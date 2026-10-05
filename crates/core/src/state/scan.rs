//! Progress of a keyspace traversal (R2.1, PLAN M1.2).
//!
//! The core never sees a cursor. It sees batches of keys arriving with a
//! progress figure, which is what lets one cursor become N for Cluster without
//! the browser above noticing (ADR-0008).

/// Where a scan has got to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ScanState {
    #[default]
    Idle,
    Running {
        scanned: u64,
        /// `DBSIZE` at the time the scan started. An estimate by nature: the
        /// keyspace moves while we walk it, and `SCAN` guarantees nothing about
        /// duplicates, so this is shown with a `~` and never as a percentage of
        /// something exact.
        estimated_total: u64,
    },
    Complete {
        total: u64,
    },
    /// `Esc` during a scan. Every in-flight operation is cancellable.
    Cancelled {
        scanned: u64,
    },
    /// The Loaded set reached its cap (ADR-0010). Scanning stopped, and this
    /// must be visible — a limit the user cannot see is one they will mistake
    /// for the whole keyspace.
    Capped {
        at: usize,
    },
    Failed {
        error: String,
    },
    /// The scan stopped because the thing it was walking changed shape under
    /// it (M5 task 3). What was loaded stays, as after `Esc`, but completion
    /// is never recorded: keys may be missing or repeated, and a list that
    /// looks finished is the one thing this state exists to prevent.
    Interrupted {
        scanned: u64,
        reason: InterruptReason,
    },
}

/// Why a scan was interrupted. The core knows the reason, never the nodes
/// behind it (ADR-0008).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum InterruptReason {
    /// A Cluster's topology changed (failover, resharding, a node joining).
    TopologyChanged,
}

impl ScanState {
    pub fn is_running(&self) -> bool {
        matches!(self, ScanState::Running { .. })
    }

    /// The status-bar readout, e.g. `scanning 41,203 of ~180,000`.
    pub fn readout(&self) -> String {
        match self {
            ScanState::Idle => String::new(),
            ScanState::Running {
                scanned,
                estimated_total,
            } => {
                format!(
                    "scanning {} of ~{}",
                    thousands(*scanned),
                    thousands(*estimated_total)
                )
            }
            ScanState::Complete { total } => format!("{} keys", thousands(*total)),
            ScanState::Cancelled { scanned } => {
                format!("stopped at {} keys", thousands(*scanned))
            }
            ScanState::Capped { at } => {
                format!(
                    "{} key limit reached — narrow the filter",
                    thousands(*at as u64)
                )
            }
            ScanState::Failed { error } => format!("scan failed: {error}"),
            ScanState::Interrupted { reason, .. } => match reason {
                InterruptReason::TopologyChanged => {
                    "the cluster changed during the scan — r to rescan".into()
                }
            },
        }
    }
}

/// Group digits so a six-figure key count is readable at a glance. Reused
/// wherever else in the app a large count needs the same treatment (the
/// Dashboard's tile figures, `render/dashboard.rs`) — one formatter, not a
/// second one written to match it by eye.
pub(crate) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_are_grouped() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(41_203), "41,203");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }

    #[test]
    fn progress_is_stated_as_an_estimate_because_that_is_what_it_is() {
        let s = ScanState::Running {
            scanned: 41_203,
            estimated_total: 180_000,
        };
        assert_eq!(s.readout(), "scanning 41,203 of ~180,000");
    }

    #[test]
    fn the_cap_readout_says_what_to_do_about_it() {
        let s = ScanState::Capped { at: 2_000_000 };
        assert!(s.readout().contains("2,000,000"));
        assert!(s.readout().contains("narrow the filter"));
    }
}
