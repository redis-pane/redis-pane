//! Bookkeeping for lazily-fetched row metadata (M4 task 4, decision 6,
//! `docs/plans/m4-perf-interaction.md`).
//!
//! The core asks for metadata with `Command::FetchMetadata { indices }` for
//! whatever rows are on screen, and asks again on every cursor move. This
//! ledger is what keeps the shell from answering every ask with a fresh
//! round trip:
//!
//! - an index with a fetch already in flight is not requested again;
//! - a row reported gone (`TYPE` -> `none`) is deliberately **not** remembered:
//!   once its fetch has finished it is asked about again whenever it is back
//!   in the window, so a key deleted and then recreated loses its gone badge
//!   as soon as it is scrolled to. The cost is a few extra `TYPE`s riding the
//!   same pipelined batch, never an extra round trip;
//! - an **epoch**, minted by the core, names which numbering of the Loaded
//!   set a fetch was issued against. A reply carries bare indices, and a
//!   rescan renumbers them (`LoadedSet::clear`, then a refill in a different
//!   `SCAN` order), so a reply that outlives its epoch would write a type,
//!   TTL and size — or a *tombstone* — onto whichever unrelated key now holds
//!   that index.
//!
//! **Staleness is decided by the core, not here.** The core bumps the epoch
//! where it renumbers (`scan_started`), stamps it on every
//! `Command::FetchMetadata`, and `metadata_batch` drops a reply whose epoch is
//! not current. An earlier version kept a shell-side counter bumped when the
//! shell handled `Command::StartScan`, and that left a window: the core only
//! renumbers when `Msg::ScanStarted` is dequeued, a `SCAN` round trip later,
//! and in between it still shows the old keys and can ask for their metadata
//! with old indices — which a shell counter would already have called
//! "current". The token makes the answer a fact about the message, not an
//! argument about timing.
//!
//! This ledger follows the epoch it sees: a `FetchMetadata` under a new epoch
//! forgets what is in flight and cancels the old epoch's fetches, which is purely an
//! optimization (no wasted round trips, no dedup entry held by a fetch whose
//! answer will be dropped) — correctness does not depend on it. A reconnect
//! also resets it, for a different reason: fetches issued on the replaced
//! client may never be answered, and an index held "in flight" by one that
//! hangs would never be asked for again.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, MutexGuard};

use redis_pane_core::command::MetadataEpoch;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct Inner {
    generation: u64,
    epoch: MetadataEpoch,
    in_flight: HashSet<usize>,
    cancel: CancellationToken,
}

/// Shared between the [`Shell`](crate::terminal) and its fetch tasks.
#[derive(Debug, Clone)]
pub struct MetadataLedger {
    inner: Arc<Mutex<Inner>>,
}

impl Default for MetadataLedger {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner {
                generation: 0,
                epoch: MetadataEpoch::default(),
                in_flight: HashSet::new(),
                cancel: CancellationToken::new(),
            })),
        }
    }
}

impl Inner {
    fn reset(&mut self) {
        self.generation += 1;
        self.in_flight.clear();
        self.cancel.cancel();
        self.cancel = CancellationToken::new();
    }
}

impl MetadataLedger {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A poisoned lock only means a fetch task panicked mid-update; the
        // sets are still structurally valid, and wedging every later fetch
        // would be worse than reading them.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Claim the indices that need a fetch: those without one already in
    /// flight. `None` when nothing is left to ask. `epoch` is
    /// the one the core stamped on the command; seeing a new one is what
    /// resets the ledger.
    pub fn claim(&self, epoch: MetadataEpoch, wanted: &[usize]) -> Option<Claim> {
        let mut inner = self.lock();
        if inner.epoch != epoch {
            inner.epoch = epoch;
            inner.reset();
        }
        let mut indices = Vec::new();
        for &i in wanted {
            if inner.in_flight.insert(i) {
                indices.push(i);
            }
        }
        if indices.is_empty() {
            return None;
        }
        Some(Claim {
            ledger: self.clone(),
            generation: inner.generation,
            epoch,
            indices,
            cancel: inner.cancel.clone(),
            settled: false,
        })
    }

    /// The connection was replaced: fetches issued on the old client may
    /// never be answered. Cancels them and forgets everything.
    pub fn reset(&self) {
        self.lock().reset();
    }

    #[cfg(test)]
    fn in_flight(&self) -> usize {
        self.lock().in_flight.len()
    }
}

/// A set of indices this task has the exclusive right to fetch. Dropping it
/// without [`Claim::finish`] — a failed fetch, a cancelled one, a panicked
/// task — releases the indices, so none can stay "in flight" forever.
#[derive(Debug)]
pub struct Claim {
    ledger: MetadataLedger,
    generation: u64,
    epoch: MetadataEpoch,
    indices: Vec<usize>,
    cancel: CancellationToken,
    settled: bool,
}

impl Claim {
    /// The epoch to echo back in the reply.
    pub fn epoch(&self) -> MetadataEpoch {
        self.epoch
    }

    pub fn indices(&self) -> &[usize] {
        &self.indices
    }

    /// Fires when a reset supersedes this claim's generation.
    pub fn cancelled(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
        self.cancel.cancelled()
    }

    /// Settle the claim, freeing its indices to be asked about again. Returns
    /// whether the reply is still current — `false` means a reset happened
    /// since the fetch was issued, and the reply must be dropped, not
    /// applied.
    pub fn finish(mut self) -> bool {
        self.settled = true;
        let mut inner = self.ledger.lock();
        if inner.generation != self.generation {
            return false;
        }
        for i in &self.indices {
            inner.in_flight.remove(i);
        }
        true
    }

    /// Whether this claim's generation is still the current one. Used to
    /// decide if a *failure* is worth surfacing: one from a superseded
    /// connection or scan is expected noise.
    pub fn is_current(&self) -> bool {
        self.ledger.lock().generation == self.generation
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if self.settled {
            return;
        }
        let mut inner = self.ledger.lock();
        if inner.generation == self.generation {
            for i in &self.indices {
                inner.in_flight.remove(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis_pane_core::{Msg, State, update};

    /// The epoch the core currently stamps, and the one after a rescan —
    /// obtained the only way a shell can, from the core.
    fn epochs() -> (MetadataEpoch, MetadataEpoch) {
        let s = State::default();
        let e0 = s.metadata_epoch;
        let (s, _) = update(s, Msg::ScanStarted { estimated_total: 1 });
        (e0, s.metadata_epoch)
    }

    #[test]
    fn an_index_in_flight_is_not_claimed_twice() {
        let l = MetadataLedger::default();
        let (e0, e1) = epochs();
        let _ = e1;
        let a = l.claim(e0, &[1, 2, 3]).unwrap();
        assert_eq!(a.indices(), [1, 2, 3]);
        let b = l.claim(e0, &[2, 3, 4, 5]).unwrap();
        assert_eq!(b.indices(), [4, 5], "only the new ones");
        assert!(l.claim(e0, &[1, 4]).is_none(), "everything already asked");
    }

    #[test]
    fn a_duplicate_within_one_request_is_claimed_once() {
        let l = MetadataLedger::default();
        let (e0, e1) = epochs();
        let _ = e1;
        assert_eq!(l.claim(e0, &[7, 7]).unwrap().indices(), [7]);
    }

    #[test]
    fn a_finished_fetch_frees_its_indices_so_a_gone_row_is_asked_again() {
        let l = MetadataLedger::default();
        let (e0, e1) = epochs();
        let _ = e1;
        let a = l.claim(e0, &[1, 2]).unwrap();
        // 2 came back gone and 1 alive; the ledger keeps neither fact.
        // While the fetch is still in flight both are deduped...
        assert!(l.claim(e0, &[1, 2]).is_none());
        assert!(a.finish());
        assert_eq!(l.in_flight(), 0);
        // ...and once it has finished, both are claimable again — the gone
        // row included, so a recreated key loses its badge on re-entry.
        assert_eq!(l.claim(e0, &[1, 2]).unwrap().indices(), [1, 2]);
    }

    #[test]
    fn a_failed_or_dropped_fetch_makes_its_indices_fetchable_again() {
        let l = MetadataLedger::default();
        let (e0, e1) = epochs();
        let _ = e1;
        let a = l.claim(e0, &[1, 2]).unwrap();
        drop(a); // error, cancellation or a panicked task
        assert_eq!(l.in_flight(), 0);
        assert_eq!(l.claim(e0, &[1, 2]).unwrap().indices(), [1, 2]);
    }

    #[test]
    fn a_new_epoch_forgets_everything_and_makes_the_old_reply_stale() {
        let l = MetadataLedger::default();
        let (e0, e1) = epochs();
        let old = l.claim(e0, &[1, 2]).unwrap();
        let first = l.claim(e0, &[9]).unwrap();
        assert!(first.finish());
        // The core renumbered (a rescan): the next ask carries the new epoch.
        // 9 is a different key now, and 1 and 2 have no fetch in flight
        // against the new numbering.
        let fresh = l.claim(e1, &[1, 2, 9]).unwrap();
        assert_eq!(fresh.indices(), [1, 2, 9]);
        assert_eq!(fresh.epoch(), e1);
        assert!(!old.is_current());
        // The old fetch lands late: reported stale, and it must not free the
        // new claim's indices. (The core drops its batch by epoch regardless.)
        assert!(!old.finish());
        assert_eq!(l.in_flight(), 3);
        assert!(fresh.finish());
    }

    #[test]
    fn a_reconnect_reset_forgets_in_flight_fetches() {
        let l = MetadataLedger::default();
        let (e0, _) = epochs();
        let hung = l.claim(e0, &[4]).unwrap();
        let done = l.claim(e0, &[5]).unwrap();
        assert!(done.finish());
        l.reset();
        assert!(!hung.is_current());
        assert_eq!(l.claim(e0, &[4, 5]).unwrap().indices(), [4, 5]);
    }

    #[test]
    fn a_dropped_stale_claim_does_not_free_the_new_generations_indices() {
        let l = MetadataLedger::default();
        let (e0, e1) = epochs();
        let _ = e1;
        let old = l.claim(e0, &[1]).unwrap();
        l.reset();
        let fresh = l.claim(e0, &[1]).unwrap();
        drop(old);
        assert!(
            l.claim(e0, &[1]).is_none(),
            "the new fetch is still in flight"
        );
        drop(fresh);
    }

    #[tokio::test]
    async fn a_reset_cancels_the_outstanding_fetches() {
        let l = MetadataLedger::default();
        let (e0, e1) = epochs();
        let _ = e1;
        let c = l.claim(e0, &[1]).unwrap();
        let waiter = tokio::spawn(async move {
            c.cancelled().await;
        });
        l.reset();
        tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
            .await
            .expect("cancelled promptly")
            .unwrap();
        // A claim made after the reset is not born cancelled.
        let fresh = l.claim(e0, &[1]).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), fresh.cancelled())
                .await
                .is_err()
        );
    }
}
