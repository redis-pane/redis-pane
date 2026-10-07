//! The record sort behind the Name sort (M6 task 4, `docs/plans/m6-fast-sort.md`).
//!
//! Sorting key names through `offsets[i]` makes every comparison two dependent
//! cache misses in a 50MB arena. This sort instead orders contiguous
//! [`Rec`]s: a record is the name's next 8 bytes (big-endian, zero-padded), how
//! many of those 8 bytes the name really has, and the key's index. Records are
//! sorted by pdqsort (chunks) and a pairwise merge, with no arena access at all.
//!
//! Names that tie on a record are *refined*: the next 8 bytes of every tied
//! name are gathered into its record (independent loads, which the CPU
//! overlaps) and each tied run is sorted again, until every run is resolved.
//! Zero padding alone would tie `"ab"` with `"ab\0"`, so a record carries its
//! real byte count and a name with fewer than 8 bytes left sorts before one
//! with 8: the byte order `name().cmp()` gives. Names that are identical
//! byte for byte keep index order (the sort is stable).
//!
//! The sort is a resumable state machine ([`NameSorter`]): a step handles at
//! most `slice` positions, so the rebuild job can run it a slice per frame
//! (the core owns no clock), and the synchronous path is the same machine
//! with an unbounded slice. There is exactly one implementation.
//!
//! As a by-product it produces `lcp`: for each position `p` in the final
//! order, the number of bytes name `p` shares with name `p - 1` (0 at
//! position 0). The fold (task 5) uses it to skip segments it already knows.

use super::loaded::{LoadedSet, be_prefix};

/// A sortable record: `key` is the name's bytes `[depth, depth + 8)`
/// big-endian and zero-padded, `tie` is `rem << 32 | index` where `rem` is
/// how many of those 8 bytes the name really has. Derived ordering compares
/// `key`, then `rem`, then the index: exactly byte order, then stability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub(super) struct Rec {
    key: u64,
    tie: u64,
}

impl Rec {
    fn idx(self) -> u32 {
        self.tie as u32
    }
    fn rem(self) -> u32 {
        (self.tie >> 32) as u32
    }
}

/// A run of positions in the record array that tie through `depth` bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    lo: u32,
    hi: u32,
    /// Bytes every name in the run is known to share: the next gather reads
    /// `[depth, depth + 8)`.
    depth: u32,
}

/// Bytes shared between two adjacent records sorted at some depth, beyond
/// that depth: at most 8.
fn common(a: Rec, b: Rec) -> u32 {
    let m = a.rem().min(b.rem());
    if a.key == b.key {
        m
    } else {
        ((a.key ^ b.key).leading_zeros() / 8).min(m)
    }
}

/// Whether two adjacent sorted records still tie: same 8 bytes, both with all
/// 8 present, so deeper bytes decide.
fn tied(a: Rec, b: Rec) -> bool {
    a.key == b.key && a.rem() == 8 && b.rem() == 8
}

/// Resumable merge over `recs[lo..hi]`: sorted chunks of `slice` merged
/// pairwise, bottom-up, ping-ponging between the two buffers.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RangeSort {
    lo: usize,
    hi: usize,
    phase: RangePhase,
    /// The sorted data currently lives in `scratch`, not `recs`.
    flipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RangePhase {
    /// Sorting chunks of `recs[lo..hi]` in place; `next` is the next start.
    Chunks {
        next: usize,
    },
    Merge(MergePass),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MergePass {
    width: usize,
    /// Output position (absolute).
    out: usize,
    /// The pair being merged: `i..mid` and `mid..pair_hi` with cursors.
    mid: usize,
    pair_hi: usize,
    i: usize,
    j: usize,
}

impl MergePass {
    fn new(lo: usize, hi: usize, width: usize) -> MergePass {
        MergePass {
            width,
            out: lo,
            mid: (lo + width).min(hi),
            pair_hi: (lo + 2 * width).min(hi),
            i: lo,
            j: (lo + width).min(hi),
        }
    }
}

impl RangeSort {
    /// Chunks of `recs[lo..hi]` are sorted first.
    fn chunks(lo: usize, hi: usize) -> RangeSort {
        RangeSort {
            lo,
            hi,
            phase: RangePhase::Chunks { next: lo },
            flipped: false,
        }
    }

    /// `recs[lo..hi]` already holds sorted chunks of `slice`.
    fn merging(lo: usize, hi: usize, slice: usize) -> RangeSort {
        RangeSort {
            lo,
            hi,
            phase: RangePhase::Merge(MergePass::new(lo, hi, slice)),
            flipped: false,
        }
    }

    /// One step of at most `slice` positions. True when `recs[lo..hi]` is
    /// sorted (and back in `recs`).
    fn step(&mut self, recs: &mut Vec<Rec>, scratch: &mut Vec<Rec>, slice: usize) -> bool {
        let (lo, hi) = (self.lo, self.hi);
        match &mut self.phase {
            RangePhase::Chunks { next } => {
                let end = next.saturating_add(slice).min(hi);
                sort_recs(&mut recs[*next..end], scratch);
                if end >= hi {
                    if hi - lo <= slice {
                        return true;
                    }
                    self.phase = RangePhase::Merge(MergePass::new(lo, hi, slice));
                } else {
                    *next = end;
                }
                false
            }
            RangePhase::Merge(m) => {
                let (src, dst): (&Vec<Rec>, &mut Vec<Rec>) = if self.flipped {
                    (&*scratch, recs)
                } else {
                    (&*recs, scratch)
                };
                let mut produced = 0;
                while produced < slice && m.out < hi {
                    // Records are never equal (the index is in the tie), so
                    // taking the right run only when it is smaller is stable.
                    let take_left = m.i < m.mid && (m.j >= m.pair_hi || src[m.i] < src[m.j]);
                    if take_left {
                        dst[m.out] = src[m.i];
                        m.i += 1;
                    } else {
                        dst[m.out] = src[m.j];
                        m.j += 1;
                    }
                    m.out += 1;
                    produced += 1;
                    if m.out == m.pair_hi {
                        let start = m.pair_hi;
                        m.mid = (start + m.width).min(hi);
                        m.pair_hi = (start + 2 * m.width).min(hi);
                        m.i = start;
                        m.j = m.mid;
                    }
                }
                if m.out >= hi {
                    self.flipped = !self.flipped;
                    let width = m.width * 2;
                    if width >= hi - lo {
                        if self.flipped {
                            if lo == 0 && hi == recs.len() {
                                std::mem::swap(recs, scratch);
                            } else {
                                recs[lo..hi].copy_from_slice(&scratch[lo..hi]);
                            }
                            self.flipped = false;
                        }
                        return true;
                    }
                    *m = MergePass::new(lo, hi, width);
                }
                false
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Phase {
    /// Filling records from the prefix column and sorting each chunk.
    Fill {
        next: usize,
    },
    /// Merging the sorted chunks.
    Merge(RangeSort),
    /// Finding the first tied runs and the depth-0 LCPs.
    Seed {
        next: usize,
    },
    /// Refining tied runs, a level at a time.
    Level,
    /// Writing the index order out.
    Emit {
        next: usize,
    },
    Done,
}

/// A tied run too large for one step: gathered, then sorted by [`RangeSort`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct Big {
    run: Run,
    /// Next position to gather; at `hi` the sort has started.
    gathered: usize,
    sort: Option<RangeSort>,
}

/// The Name sort as a resumable state machine. See the module docs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct NameSorter {
    n: usize,
    slice: usize,
    recs: Vec<Rec>,
    scratch: Vec<Rec>,
    lcp: Vec<u32>,
    work: Vec<Run>,
    next_work: Vec<Run>,
    /// An open tied run start while scanning pairs.
    carry: Option<usize>,
    /// The next run of `work` to refine, and the big run in progress.
    cursor: usize,
    big: Option<Big>,
    level: u32,
    phase: Phase,
}

impl NameSorter {
    /// A sorter for `n` positions of an order, stepping `slice` positions at
    /// a time.
    pub(super) fn new(n: usize, slice: usize) -> NameSorter {
        NameSorter {
            n,
            slice: slice.max(1),
            recs: vec![Rec::default(); n],
            scratch: Vec::new(),
            lcp: vec![0; n],
            work: Vec::new(),
            next_work: Vec::new(),
            carry: None,
            cursor: 0,
            big: None,
            level: 0,
            phase: if n < 2 {
                Phase::Emit { next: 0 }
            } else {
                Phase::Fill { next: 0 }
            },
        }
    }

    /// Bytes per position the sorter holds beside the order.
    pub(super) fn heap_bytes(&self) -> usize {
        (self.recs.capacity() + self.scratch.capacity()) * std::mem::size_of::<Rec>()
            + self.lcp.capacity() * 4
            + (self.work.capacity() + self.next_work.capacity()) * std::mem::size_of::<Run>()
    }

    /// The LCP of each position with its predecessor, once [`NameSorter::step`]
    /// has returned true.
    pub(super) fn into_lcp(self) -> Vec<u32> {
        self.lcp
    }

    /// Coarse progress, 0 to 1, for the `rebuilding N%` readout.
    pub(super) fn fraction(&self) -> f32 {
        let n = self.n.max(1) as f32;
        match &self.phase {
            Phase::Fill { next } => 0.15 * (*next as f32 / n),
            Phase::Merge(r) => {
                let done = (r.hi - r.lo) as f32;
                0.15 + 0.35
                    * if done > 0.0 {
                        self.merge_fraction(r)
                    } else {
                        1.0
                    }
            }
            Phase::Seed { next } => 0.5 + 0.1 * (*next as f32 / n),
            Phase::Level => 0.6 + 0.35 * (1.0 - 1.0 / (1.0 + self.level as f32)),
            Phase::Emit { next } => 0.95 + 0.05 * (*next as f32 / n),
            Phase::Done => 1.0,
        }
    }

    fn merge_fraction(&self, r: &RangeSort) -> f32 {
        let len = (r.hi - r.lo).max(1);
        let passes = (len.div_ceil(self.slice))
            .next_power_of_two()
            .trailing_zeros()
            .max(1) as f32;
        match &r.phase {
            RangePhase::Merge(m) => {
                let pass = (m.width / self.slice).max(1).trailing_zeros() as f32;
                ((pass + (m.out - r.lo) as f32 / len as f32) / passes).clamp(0.0, 1.0)
            }
            RangePhase::Chunks { .. } => 0.0,
        }
    }

    /// Which part is next, for the perf suite's per-stage figures.
    pub(super) fn phase_name(&self) -> &'static str {
        match &self.phase {
            Phase::Fill { .. } => "sort chunk",
            Phase::Merge(_) => "sort merge",
            Phase::Seed { .. } => "sort seed",
            Phase::Level => "sort refine",
            Phase::Emit { .. } => "sort emit",
            Phase::Done => "sort done",
        }
    }

    /// Advance by at most one slice of positions. `order` is the order being
    /// sorted: read while filling, written once at the end. Returns true when
    /// `order` is sorted and the LCPs are final.
    pub(super) fn step(&mut self, keys: &LoadedSet, order: &mut [u32]) -> bool {
        debug_assert_eq!(order.len(), self.n);
        let (n, slice) = (self.n, self.slice);
        match &mut self.phase {
            Phase::Fill { next } => {
                let (lo, hi) = (*next, next.saturating_add(slice).min(n));
                for (rec, idx) in self.recs[lo..hi].iter_mut().zip(&order[lo..hi]) {
                    let idx = *idx;
                    let len = keys.len_of(idx as usize).unwrap_or(0);
                    let prefix = keys.prefix(idx as usize).unwrap_or(0);
                    *rec = Rec {
                        key: prefix,
                        tie: ((len.min(8) as u64) << 32) | idx as u64,
                    };
                }
                sort_recs(&mut self.recs[lo..hi], &mut self.scratch);
                if hi >= n {
                    if n <= slice {
                        self.phase = Phase::Seed { next: 1 };
                    } else {
                        self.scratch.resize(n, Rec::default());
                        self.phase = Phase::Merge(RangeSort::merging(0, n, slice));
                    }
                } else {
                    *next = hi;
                }
            }
            Phase::Merge(r) => {
                if r.step(&mut self.recs, &mut self.scratch, slice) {
                    self.scratch = Vec::new();
                    self.phase = Phase::Seed { next: 1 };
                }
            }
            Phase::Seed { next } => {
                let (lo, hi) = (*next, next.saturating_add(slice).min(n));
                *next = hi;
                let done = hi >= n;
                self.scan_pairs(lo, hi, 0);
                if done {
                    self.flush_carry(n, 0);
                    self.phase = Phase::Level;
                    self.cursor = 0;
                    self.level = 0;
                    std::mem::swap(&mut self.work, &mut self.next_work);
                    self.next_work.clear();
                }
            }
            Phase::Level => self.level_step(keys),
            Phase::Emit { next } => {
                let (lo, hi) = (*next, next.saturating_add(slice).min(n));
                if n >= 2 {
                    for (o, rec) in order[lo..hi].iter_mut().zip(&self.recs[lo..hi]) {
                        *o = rec.idx();
                    }
                }
                if hi >= n {
                    self.recs = Vec::new();
                    self.work = Vec::new();
                    self.next_work = Vec::new();
                    self.phase = Phase::Done;
                    return true;
                }
                *next = hi;
            }
            Phase::Done => return true,
        }
        false
    }

    /// LCPs for pairs `(p - 1, p)`, `p` in `from..to`, of records sorted at
    /// `depth`, collecting tied runs into `next_work` (a run still open at
    /// `to` stays in `carry`).
    fn scan_pairs(&mut self, from: usize, to: usize, depth: u32) {
        for p in from..to {
            let (a, b) = (self.recs[p - 1], self.recs[p]);
            if tied(a, b) {
                self.lcp[p] = depth + 8;
                self.carry.get_or_insert(p - 1);
            } else {
                self.lcp[p] = depth + common(a, b);
                if let Some(s) = self.carry.take() {
                    self.next_work.push(Run {
                        lo: s as u32,
                        hi: p as u32,
                        depth: depth + 8,
                    });
                }
            }
        }
    }

    fn flush_carry(&mut self, end: usize, depth: u32) {
        if let Some(s) = self.carry.take() {
            self.next_work.push(Run {
                lo: s as u32,
                hi: end as u32,
                depth: depth + 8,
            });
        }
    }

    /// Refine tied runs: up to `slice` positions gathered and sorted.
    fn level_step(&mut self, keys: &LoadedSet) {
        let slice = self.slice;
        // A big run in progress owns the whole step.
        if let Some(mut b) = self.big.take() {
            let Run { lo, hi, depth } = b.run;
            if let Some(sort) = &mut b.sort {
                if sort.step(&mut self.recs, &mut self.scratch, slice) {
                    self.scratch = Vec::new();
                    self.finish_run(lo as usize, hi as usize, depth);
                    return;
                }
            } else {
                let end = b.gathered.saturating_add(slice).min(hi as usize);
                gather(keys, &mut self.recs[b.gathered..end], depth as usize);
                b.gathered = end;
                if end >= hi as usize {
                    if self.scratch.len() < self.n {
                        self.scratch.resize(self.n, Rec::default());
                    }
                    b.sort = Some(RangeSort::chunks(lo as usize, hi as usize));
                }
            }
            self.big = Some(b);
            return;
        }
        let mut budget = slice;
        loop {
            if self.cursor >= self.work.len() {
                if self.next_work.is_empty() {
                    self.phase = Phase::Emit { next: 0 };
                    return;
                }
                std::mem::swap(&mut self.work, &mut self.next_work);
                self.next_work.clear();
                self.cursor = 0;
                self.level += 1;
                if budget < slice {
                    return;
                }
                continue;
            }
            let run = self.work[self.cursor];
            let len = (run.hi - run.lo) as usize;
            if len > slice {
                if budget < slice {
                    return;
                }
                self.cursor += 1;
                self.big = Some(Big {
                    run,
                    gathered: run.lo as usize,
                    sort: None,
                });
                return;
            }
            if len > budget {
                return;
            }
            let (lo, hi) = (run.lo as usize, run.hi as usize);
            gather(keys, &mut self.recs[lo..hi], run.depth as usize);
            sort_recs(&mut self.recs[lo..hi], &mut self.scratch);
            self.cursor += 1;
            budget -= len;
            self.finish_run(lo, hi, run.depth);
            if budget == 0 {
                return;
            }
        }
    }

    /// A tied run `lo..hi` whose records were gathered at `depth` and sorted:
    /// set its LCPs and queue the subruns that still tie.
    fn finish_run(&mut self, lo: usize, hi: usize, depth: u32) {
        self.scan_pairs(lo + 1, hi, depth);
        self.flush_carry(hi, depth);
    }
}

/// Runs shorter than this are sorted by pdqsort: a radix pass costs a
/// histogram and a scatter, which small runs never win back.
const RADIX_MIN: usize = 1024;
/// A run with more varying bytes than this goes to pdqsort: each varying
/// byte is another full pass over the run.
const RADIX_MAX_BYTES: usize = 3;

/// Sort `recs` by (key, tie) given they are in index order within equal
/// (key, rem): the order of a stable sort by (key, rem). Radix over the bytes
/// that actually vary (a deep keyspace's records differ in a byte or two),
/// pdqsort otherwise. `tmp` is scratch, grown as needed.
fn sort_recs(recs: &mut [Rec], tmp: &mut Vec<Rec>) {
    let n = recs.len();
    if n < RADIX_MIN {
        recs.sort_unstable();
        return;
    }
    // Which bytes vary: one cheap pass.
    let (first_key, first_rem) = (recs[0].key, recs[0].rem());
    let (mut diff, mut rem_diff) = (0u64, 0u32);
    for r in recs.iter() {
        diff |= r.key ^ first_key;
        rem_diff |= r.rem() ^ first_rem;
    }
    // Digits, least significant first: rem, then key bytes from the low end.
    let mut digits: [u8; 9] = [0; 9];
    let mut count = 0;
    if rem_diff != 0 {
        digits[count] = 8; // 8 stands for the rem
        count += 1;
    }
    for b in 0..8u8 {
        if (diff >> (8 * b)) & 0xff != 0 {
            digits[count] = b;
            count += 1;
        }
    }
    if count == 0 {
        return; // all equal on (key, rem): already in index order
    }
    if count > RADIX_MAX_BYTES {
        recs.sort_unstable();
        return;
    }
    let digits = &digits[..count];
    let byte = |r: &Rec, d: u8| -> usize {
        if d == 8 {
            (r.rem() & 0xff) as usize
        } else {
            ((r.key >> (8 * d as u32)) & 0xff) as usize
        }
    };
    let mut hist = [[0u32; 256]; 3];
    for r in recs.iter() {
        for (h, d) in hist.iter_mut().zip(digits) {
            h[byte(r, *d)] += 1;
        }
    }
    if tmp.len() < n {
        tmp.resize(n, Rec::default());
    }
    let tmp = &mut tmp[..n];
    let mut in_tmp = false;
    for (h, d) in hist.iter_mut().zip(digits) {
        let mut sum = 0u32;
        for slot in h.iter_mut() {
            let c = *slot;
            *slot = sum;
            sum += c;
        }
        let (src, dst): (&[Rec], &mut [Rec]) = if in_tmp {
            (&*tmp, &mut *recs)
        } else {
            (&*recs, &mut *tmp)
        };
        for r in src {
            let slot = &mut h[byte(r, *d)];
            dst[*slot as usize] = *r;
            *slot += 1;
        }
        in_tmp = !in_tmp;
    }
    if in_tmp {
        recs.copy_from_slice(tmp);
    }
}

/// Fill each record's key and `rem` from its name's bytes `[depth, depth+8)`.
///
/// Done in batches: first every name's span, then every name's bytes, so the
/// arena loads of a batch do not wait on each other.
fn gather(keys: &LoadedSet, recs: &mut [Rec], depth: usize) {
    const BATCH: usize = 32;
    for batch in recs.chunks_mut(BATCH) {
        let mut spans = [(0usize, 0usize); BATCH];
        for (s, r) in spans.iter_mut().zip(batch.iter()) {
            *s = keys.span(r.idx() as usize);
        }
        for (r, (start, len)) in batch.iter_mut().zip(spans) {
            let idx = r.idx();
            let (key, rem) = if len >= depth + 8 {
                let b = keys.arena_at(start + depth, 8);
                (u64::from_be_bytes(b.try_into().unwrap_or([0; 8])), 8)
            } else if len > depth {
                let tail = keys.arena_at(start + depth, len - depth);
                (be_prefix(tail), tail.len())
            } else {
                (0, 0)
            };
            *r = Rec {
                key,
                tie: ((rem as u64) << 32) | idx as u64,
            };
        }
    }
}

/// Sort `order` (key indices) into byte order of the key names, ties by
/// index value. Returns each position's LCP with its predecessor. This is
/// the one implementation: the rebuild job steps the same machine.
pub(super) fn sort_by_name(keys: &LoadedSet, order: &mut [u32]) -> Vec<u32> {
    let mut sorter = NameSorter::new(order.len(), usize::MAX);
    while !sorter.step(keys, order) {}
    sorter.into_lcp()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tiny deterministic generator (xorshift64*), so failures reproduce.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    fn naive(keys: &LoadedSet, order: &[u32]) -> (Vec<u32>, Vec<u32>) {
        let name = |i: u32| keys.name(i as usize).unwrap();
        let mut want = order.to_vec();
        want.sort_by(|a, b| name(*a).cmp(name(*b)).then(a.cmp(b)));
        let lcp = (0..want.len())
            .map(|p| {
                if p == 0 {
                    0
                } else {
                    let (x, y) = (name(want[p - 1]), name(want[p]));
                    x.iter().zip(y).take_while(|(a, b)| a == b).count() as u32
                }
            })
            .collect();
        (want, lcp)
    }

    fn set_of(names: &[Vec<u8>]) -> LoadedSet {
        let mut s = LoadedSet::default();
        for n in names {
            assert!(s.push(n));
        }
        s
    }

    fn check(names: &[Vec<u8>], subset_every: usize, what: &str) {
        let keys = set_of(names);
        let order: Vec<u32> = (0..names.len() as u32)
            .filter(|i| (*i as usize).is_multiple_of(subset_every))
            .collect();
        let (want, want_lcp) = naive(&keys, &order);
        for slice in [usize::MAX, 1, 2, 3, 7, 64, 1000] {
            let mut got = order.clone();
            let mut sorter = NameSorter::new(got.len(), slice);
            let mut steps = 0;
            while !sorter.step(&keys, &mut got) {
                steps += 1;
                assert!(
                    steps < 5_000_000,
                    "{what}: slice {slice} does not terminate"
                );
            }
            assert_eq!(got, want, "{what}: order differs (slice {slice})");
            assert_eq!(
                sorter.into_lcp(),
                want_lcp,
                "{what}: lcp differs (slice {slice})"
            );
        }
    }

    fn random_names(rng: &mut Rng, n: usize, alphabet: &[u8], max_len: usize) -> Vec<Vec<u8>> {
        (0..n)
            .map(|_| {
                let len = rng.below(max_len + 1);
                (0..len)
                    .map(|_| alphabet[rng.below(alphabet.len())])
                    .collect()
            })
            .collect()
    }

    #[test]
    fn matches_the_comparison_sort_on_tiny_alphabets_with_zero_bytes() {
        // Names that are prefixes of each other, padded with \0, empty:
        // exactly where a zero-padded prefix alone would tie wrongly.
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        for round in 0..30 {
            let names = random_names(&mut rng, 40 + round * 13, &[0, 1, b'a'], 20);
            check(&names, 1, "tiny alphabet");
            check(&names, 3, "tiny alphabet subset");
        }
    }

    #[test]
    fn ab_and_ab_nul_are_not_a_tie() {
        let names: Vec<Vec<u8>> = [
            &b"ab\0"[..],
            b"ab",
            b"ab\0\0",
            b"",
            b"\0",
            b"ab\0\0\0\0\0\0",
            b"ab\0\0\0\0\0\0\0",
            b"ab",
            b"abcdefgh",
            b"abcdefgh\0",
            b"abcdefg",
            b"abcdefg\0",
        ]
        .iter()
        .map(|n| n.to_vec())
        .collect();
        check(&names, 1, "zero pad trap");
    }

    #[test]
    fn matches_on_long_shared_prefixes() {
        let mut rng = Rng(42);
        for shared in [7usize, 8, 9, 15, 16, 17, 24, 40] {
            let head: Vec<u8> = (0..shared).map(|i| b'a' + (i % 7) as u8).collect();
            let names: Vec<Vec<u8>> = random_names(&mut rng, 300, b"xyz\0", 14)
                .into_iter()
                .map(|t| [head.clone(), t].concat())
                .collect();
            check(&names, 1, &format!("shared {shared}"));
        }
    }

    #[test]
    fn matches_on_identical_names_and_whole_byte_range() {
        let mut rng = Rng(7);
        let bytes: Vec<u8> = (0..=255).collect();
        let mut names = random_names(&mut rng, 400, &bytes, 25);
        // Duplicates, including long ones: equal names keep index order.
        for i in 0..60 {
            let dup = names[i * 3].clone();
            names.push(dup);
        }
        check(&names, 1, "full byte range");
    }

    #[test]
    fn matches_on_deep_keyspace_shapes() {
        let mut rng = Rng(99);
        let mut names: Vec<Vec<u8>> = (0..2000)
            .map(|i| format!("app:{}:tenant:{}:user:{i:08}:session", i % 7, i % 211).into_bytes())
            .collect();
        for i in (1..names.len()).rev() {
            names.swap(i, rng.below(i + 1));
        }
        check(&names, 1, "deep");
    }

    #[test]
    fn matches_with_runs_large_enough_for_radix() {
        let mut rng = Rng(2024);
        // Few varying bytes (radix) and many (pdqsort), at sizes past RADIX_MIN.
        let mut deep: Vec<Vec<u8>> = (0..6000)
            .map(|i| format!("app:{}:tenant:{}:user:{i:08}:s", i % 3, i % 17).into_bytes())
            .collect();
        for i in (1..deep.len()).rev() {
            deep.swap(i, rng.below(i + 1));
        }
        check(&deep, 1, "deep, radix-sized");
        let mut digits = random_names(&mut rng, 5000, b"0123456789", 6);
        digits.extend(random_names(&mut rng, 3000, &[0, 1, b'a', 255], 9));
        check(&digits, 1, "short, radix-sized");
    }

    #[test]
    fn empty_and_single_orders() {
        let keys = set_of(&[b"a".to_vec()]);
        let mut empty: Vec<u32> = vec![];
        assert!(sort_by_name(&keys, &mut empty).is_empty());
        let mut one = vec![0u32];
        assert_eq!(sort_by_name(&keys, &mut one), vec![0]);
        assert_eq!(one, vec![0]);
    }

    #[test]
    fn a_step_touches_at_most_one_slice_of_positions() {
        // Count the positions each step gathers or sorts via the recs it
        // can change: observable as the number of steps being at least
        // n / slice and growing as the slice shrinks.
        let mut rng = Rng(5);
        let names = random_names(&mut rng, 2000, b"ab", 30);
        let keys = set_of(&names);
        let steps = |slice: usize| {
            let mut order: Vec<u32> = (0..names.len() as u32).collect();
            let mut s = NameSorter::new(order.len(), slice);
            let mut n = 0;
            while !s.step(&keys, &mut order) {
                n += 1;
            }
            n
        };
        assert!(steps(100) > steps(1000));
        assert!(steps(1000) >= 2000 / 1000);
    }

    #[test]
    fn prefix_column_is_big_endian_and_zero_padded() {
        let keys = set_of(&[b"ab".to_vec(), b"abcdefghij".to_vec(), vec![]]);
        assert_eq!(keys.prefix(0), Some(0x6162_0000_0000_0000));
        assert_eq!(keys.prefix(1), Some(0x6162_6364_6566_6768));
        assert_eq!(keys.prefix(2), Some(0));
        assert_eq!(keys.len_of(1), Some(10));
    }

    /// Not a gate: where the time goes at 1M deep random names, per phase.
    /// `cargo test -p redis-pane-core --release --lib sort::tests::phase_times -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn phase_times() {
        use std::time::Instant;
        let mut rng = Rng(1);
        let n = 1_000_000usize;
        let mut ids: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            ids.swap(i, rng.below(i + 1));
        }
        let mut keys = LoadedSet::default();
        for i in ids {
            keys.push(format!("app:{}:tenant:{}:user:{i:08}:session", i % 7, i % 211).as_bytes());
        }
        for slice in [usize::MAX, 32_768] {
            let mut order: Vec<u32> = (0..n as u32).collect();
            let mut s = NameSorter::new(n, slice);
            let mut totals: Vec<(&str, std::time::Duration, std::time::Duration, usize)> = vec![];
            let all = Instant::now();
            loop {
                let name = s.phase_name();
                let t = Instant::now();
                let done = s.step(&keys, &mut order);
                let d = t.elapsed();
                match totals.iter_mut().find(|x| x.0 == name) {
                    Some(x) => {
                        x.1 += d;
                        x.2 = x.2.max(d);
                        x.3 += 1;
                    }
                    None => totals.push((name, d, d, 1)),
                }
                if done {
                    break;
                }
            }
            println!("slice {slice}: total {:?}", all.elapsed());
            for (name, total, worst, steps) in totals {
                println!("  {name}: {total:?} over {steps} steps, worst {worst:?}");
            }
        }
    }
}
