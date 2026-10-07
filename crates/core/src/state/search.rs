//! The filter's fast path (M6 task 6, `docs/plans/m6-filter-fast-path.md`).
//!
//! A glob with no `*` or `?` is a case-insensitive substring search
//! ([`matches`](super::view::matches)). Offering every key to `matches` is a
//! per-key `windows` scan; this searches the *arena* instead, which holds the
//! names back to back: it looks for the pattern's first byte in both cases
//! (`memchr2`, vectorised), verifies a candidate case-insensitively, and walks
//! a key cursor forward so each hit is attributed to the key that holds it.
//! No lowercase copy of the arena is made.
//!
//! Equivalence with `matches` is the contract and is property-tested below:
//! ASCII-only case folding (`eq_ignore_ascii_case`, so bytes >= 0x80 never
//! fold), a hit that straddles two names does not count, and a key is reported
//! once however often it matches. Everything else — wildcards, fuzzy — keeps
//! the per-key path.

use super::loaded::LoadedSet;
use super::view::{FilterMode, matches};

/// A case-insensitive substring search over key names.
pub(super) struct Substring {
    /// The pattern, ASCII-lowercased. Never empty.
    lower: Vec<u8>,
    first: (u8, u8),
}

impl Substring {
    /// The searcher for `pattern`, when `matches` would treat it as a
    /// substring search: glob mode, non-empty, no `*` or `?`.
    pub(super) fn new(pattern: &str, mode: FilterMode) -> Option<Self> {
        let p = pattern.as_bytes();
        if mode != FilterMode::Glob || p.is_empty() || p.iter().any(|b| matches!(b, b'*' | b'?')) {
            return None;
        }
        let lower = p.to_ascii_lowercase();
        let first = (lower[0], lower[0].to_ascii_uppercase());
        Some(Self { lower, first })
    }

    /// Call `f` with every key in `lo..hi` whose name contains the pattern,
    /// in ascending index order, once each.
    pub(super) fn for_each(
        &self,
        keys: &LoadedSet,
        lo: usize,
        hi: usize,
        mut f: impl FnMut(usize),
    ) {
        let hi = hi.min(keys.len());
        if lo >= hi {
            return;
        }
        // Names are contiguous in the arena, so keys `lo..hi` are one region.
        let (start, _) = keys.span(lo);
        let (last_start, last_len) = keys.span(hi - 1);
        let hay = keys.arena_at(start, last_start + last_len - start);
        let m = self.lower.len();
        let mut k = lo;
        // Where to resume looking for a candidate, relative to `start`.
        let mut pos = 0usize;
        while pos < hay.len() {
            let Some(h) = memchr::memchr2(self.first.0, self.first.1, &hay[pos..]) else {
                return;
            };
            let p = pos + h;
            // The key holding byte `p`: advance past every key that ends at or
            // before it (this also steps over empty names). `p` is inside the
            // region, so this stops at or before `hi - 1`.
            let key_end = loop {
                let (o, l) = keys.span(k);
                let end = o + l - start;
                if end > p {
                    break end;
                }
                k += 1;
            };
            if p + m > key_end {
                // Runs into the next name, and so would every later start in
                // this one: no hit here.
                pos = key_end;
            } else if hay[p..p + m]
                .iter()
                .zip(&self.lower)
                .all(|(a, b)| a.to_ascii_lowercase() == *b)
            {
                f(k);
                k += 1;
                pos = key_end;
            } else {
                pos = p + 1;
            }
        }
    }
}

/// Call `f` with every key in `lo..hi` that passes the filter, ascending, via
/// the substring search when `matches` would run one and per key otherwise.
/// An empty filter passes every key.
pub(super) fn filter_range(
    keys: &LoadedSet,
    filter: &str,
    mode: FilterMode,
    lo: usize,
    hi: usize,
    mut f: impl FnMut(usize),
) {
    let hi = hi.min(keys.len());
    if filter.is_empty() {
        (lo..hi).for_each(f);
    } else if let Some(s) = Substring::new(filter, mode) {
        s.for_each(keys, lo, hi, f);
    } else {
        for i in lo..hi {
            if keys.name(i).is_some_and(|n| matches(n, filter, mode)) {
                f(i);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
    }

    /// A small alphabet that forces collisions: both cases, a separator, and
    /// bytes at and above 0x80 (including 0xC1 / 0xE1, whose low bits equal
    /// 'A' / 'a' and so would fold under a careless `| 0x20`).
    const ALPHA: [u8; 9] = [b'a', b'A', b'b', b'B', b':', b'1', 0xC1, 0xE1, 0x00];

    fn word(rng: &mut Rng, max: usize) -> Vec<u8> {
        let n = rng.below(max + 1);
        (0..n).map(|_| ALPHA[rng.below(ALPHA.len())]).collect()
    }

    fn arena(rng: &mut Rng) -> LoadedSet {
        let mut s = LoadedSet::default();
        for _ in 0..rng.below(40) {
            s.push(&word(rng, 8));
        }
        s
    }

    fn reference(keys: &LoadedSet, pat: &str, lo: usize, hi: usize) -> Vec<usize> {
        (lo..hi)
            .filter(|&i| matches(keys.name(i).unwrap(), pat, FilterMode::Glob))
            .collect()
    }

    fn fast(keys: &LoadedSet, pat: &str, lo: usize, hi: usize) -> Vec<usize> {
        let mut out = Vec::new();
        filter_range(keys, pat, FilterMode::Glob, lo, hi, |i| out.push(i));
        out
    }

    /// Patterns must be `&str`; keep to ASCII plus the two high bytes' valid
    /// UTF-8 neighbours (`é` is 0xC3 0xA9), and also test raw high bytes on the
    /// arena side.
    fn pattern(rng: &mut Rng) -> String {
        const P: [&str; 8] = ["a", "A", "b", ":", "1", "é", "\u{0}", "B"];
        (0..rng.below(5)).map(|_| P[rng.below(P.len())]).collect()
    }

    #[test]
    fn the_search_equals_matches_over_random_arenas_and_patterns() {
        let mut rng = Rng(0x9E3779B97F4A7C15);
        for _ in 0..4000 {
            let keys = arena(&mut rng);
            let pat = pattern(&mut rng);
            assert_eq!(
                fast(&keys, &pat, 0, keys.len()),
                reference(&keys, &pat, 0, keys.len()),
                "pattern {pat:?}"
            );
        }
    }

    #[test]
    fn any_key_range_equals_matches_so_the_search_resumes_at_key_boundaries() {
        let mut rng = Rng(0xDEADBEEFCAFEF00D);
        for _ in 0..4000 {
            let keys = arena(&mut rng);
            let pat = pattern(&mut rng);
            let n = keys.len();
            let lo = rng.below(n + 1);
            let hi = lo + rng.below(n - lo + 1);
            assert_eq!(
                fast(&keys, &pat, lo, hi),
                reference(&keys, &pat, lo, hi),
                "pattern {pat:?} range {lo}..{hi}"
            );
        }
    }

    #[test]
    fn stepping_in_slices_gives_the_whole_answer() {
        let mut rng = Rng(42);
        for _ in 0..1000 {
            let keys = arena(&mut rng);
            let pat = pattern(&mut rng);
            let slice = 1 + rng.below(5);
            let mut got = Vec::new();
            let mut at = 0;
            while at < keys.len() {
                let end = (at + slice).min(keys.len());
                filter_range(&keys, &pat, FilterMode::Glob, at, end, |i| got.push(i));
                at = end;
            }
            assert_eq!(got, reference(&keys, &pat, 0, keys.len()));
        }
    }

    fn check(names: &[&[u8]], pat: &str) {
        let mut keys = LoadedSet::default();
        for n in names {
            keys.push(n);
        }
        assert_eq!(
            fast(&keys, pat, 0, keys.len()),
            reference(&keys, pat, 0, keys.len()),
            "{pat:?} over {names:?}"
        );
    }

    #[test]
    fn the_named_edge_cases() {
        // A hit spanning two adjacent names: "ab" + "cd" holds "bc" in the arena.
        check(&[b"ab", b"cd"], "bc");
        check(&[b"ab", b"cd", b"bc"], "bc");
        // Boundaries: start of a name, end of a name, whole name.
        check(&[b"abc", b"abc", b"xabc", b"abcx"], "abc");
        // Repeated hits in one key are one key.
        check(&[b"aaaa", b"abab", b"b"], "a");
        check(&[b"abababab"], "ab");
        // Empty names, and a pattern longer than any name.
        check(&[b"", b"a", b"", b"", b"ab"], "a");
        check(&[b"", b""], "a");
        check(&[b"a", b"b"], "abc");
        // Case: ASCII folds, bytes >= 0x80 do not.
        check(&[b"USER", b"User", b"user", b"usEr"], "uSeR");
        check(&[&[0xC1], &[0xE1], b"A", b"a"], "a");
        check(&[&[0xC1], &[0xE1], b"A", b"a"], "A");
        // Single-byte patterns.
        check(&[b"x", b"y", b"", b"xx"], "x");
        // A key that is the pattern exactly, next to near misses.
        check(&[b"user", b"use", b"users", b"seru"], "user");
        // NUL bytes in names and in the pattern.
        check(&[b"a\0b", b"\0"], "\0");
    }

    #[test]
    fn what_takes_the_fast_path_is_exactly_the_wildcard_free_glob() {
        assert!(Substring::new("abc", FilterMode::Glob).is_some());
        assert!(Substring::new("", FilterMode::Glob).is_none());
        assert!(Substring::new("a*", FilterMode::Glob).is_none());
        assert!(Substring::new("a?c", FilterMode::Glob).is_none());
        assert!(Substring::new("abc", FilterMode::Fuzzy).is_none());
    }
}
