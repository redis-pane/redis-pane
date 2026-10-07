# M6 task 5: Fast fold

Status: **planned.**

## Context

[`m6-planning.md`](m6-planning.md), row 5. `Tree::rebuild` (`crates/core/src/state/tree.rs`)
walks the name-sorted view and does two things for each key:

1. splits its name into segments;
2. compares those segments byte by byte with the previous key's, to find how many group levels
   they share.

Over a pre-sorted arena this takes 38 ms at 1M keys. At random order it takes 139–205 ms,
because every key's name is a cache miss in name order. Task 3 makes the fold resumable, and
task 4's sort emits each key's LCP with its predecessor.

## Decisions

1. **Use the LCP to find the shared groups without comparing bytes.** The number of shared
   segments is the number of separators inside the shared prefix (the LCP). Count them in the
   current key's own bytes, up to the LCP, using bytes this key already loads to split its own
   segments. The comparison against the previous key's segments then goes away, and so does
   half of the arena traffic.

   The LCP must reach the fold alongside the view order. Either `KeyView` keeps a parallel
   `lcp: Vec<u16>` (2 MB at 1M), or the job carries it from its Sort stage to its Fold stage.
   Choose one and record why. A narrowed or extended view has no fresh LCP, so the fold falls
   back to today's byte comparison there.
2. **Hide the remaining miss per key.** Each key's name still has to be read once. Measure
   these candidates:
   - a separate gather pass that copies each name's segment boundaries, or its first N bytes,
     into a contiguous buffer, in slices;
   - a software lookahead that touches the name `k` rows ahead in the existing loop.

   Keep whichever measures faster; the code stays portable, with no `unsafe` arch intrinsics
   unless the gain is large and the measurements are recorded.
3. **Target:** the fold alone at 1M keys, deep names, random order, is ≤ 60 ms of total work.
   The time until the new list appears for tree toggle is ≤ 250 ms total, which is M6's goal.

## Files touched

| File | Change |
|---|---|
| `crates/core/src/state/tree.rs` | LCP-driven shared-segment count; the gather or lookahead |
| `crates/core/src/state/{view,mod}.rs` | carrying the LCP |
| `crates/core/tests/perf.rs` | the fold-alone and toggle time-to-list measurements |

## Testing

- **Equivalence:** the fold with LCP produces exactly the rows, descendant counts, expanded
  flags and inverse that today's fold produces. Test it property-style, including:
  - collapsed groups, with the `cache:` / `cache:user:` regression from the existing comment;
  - keys with empty segments (`a::b`), a trailing separator, and no separator at all;
  - the fallback path with no LCP.
- **Task 3's job equivalence tests pass unchanged.**
- **Perf:** fold alone, time until the new list appears for collapse (fold only) and for tree
  toggle. Report local and CI figures.

## Phase log (working notes; folded into the Outcome at the end)

### Phase 1: baseline on main (local, load avg ~2.2-2.7, noisy machine)

| Measure (1M random_deep unless noted) | Local |
|---|---|
| fold alone (`Tree::rebuild`) | 207 ms (sorted 39.5, random_flat 118) |
| collapse / expand time-to-swap | 171-183 / 200 ms; worst step 6.9 / 7.4 ms |
| tree toggle time-to-swap from scan order | 309 ms (time_to_new_list) |
| tree toggle from Name-sorted flat (profile) | 225 ms; worst step 8.5 ms |
| whole-scan fold (tree mode) | total 1.43 s, scan end to level list 441 ms, worst page 74 ms (3 pages over 16 ms: noise, `fold` step 40 ms) |
| memory | LoadedSet 64.0 + View 7.6 + Tree 35.8 = 107.4 MB; job peak: 124.4 MB during, 104.9 MB after |
| slice calibration, worst fold step @ 8192/16384/24576/32768/49152/65536 | 2.3 / 4.2 / 5.8 / 8.1 / 11.0 / 14.8 ms |

Two perf tests failed on this noisy baseline run (`toggle_tree_at_1m_keys_random_deep`: a 34.9 ms
fold step; `whole_scan_fold..._random_deep`: 74 ms page): the step normally measures 8 ms, so these
are scheduler noise, not code.

Throwaway profile (1M random_deep, Name-sorted view, median of 5, no collapsed prefix):

| Slice of the work | Cumulative time |
|---|---|
| A. read one byte of each name in view order (the miss floor; loads are independent, so the core overlaps them) | 6.5 ms |
| B. + `split_segments` (byte loop that depends on the loaded line) | 74 ms |
| C. + shared-segment byte comparison | 123 ms (compare itself ~49 ms) |
| D. full key pass (rows, counts, open_groups) | 178 ms (rows/counts/groups ~55 ms) |
| fix-up pass | 4 ms |
| D with one collapsed prefix: key pass | 294 ms (the per-separator `HashSet<String>` probes in `ancestor_collapsed` cost ~115 ms, eight SipHash probes per key) |

So: the split is 74 ms of which most is the *serialised* miss (A shows the misses are cheap when
independent), the comparison ~49 ms, the row pushes ~55 ms, and with any collapsed prefix the
ancestor probes are the single biggest cost.

### Phase 2: LCP-driven fold

**Shared segments.** `shared` = the number of the previous key's separators that sit before byte
`lcp` (`s < lcp`). Equivalent to the old comparison: segment *i* is equal in both names exactly when
both carry the same bytes through its terminating separator, i.e. when that separator is inside the
shared prefix; a separator *at* the LCP is the first differing byte or past it (so `a:b`, `a:c`
share one segment, `a`, `a:` share none). A key that is a prefix of its predecessor (`lcp = len`)
shares the separators it has; `a::b` empty segments, a trailing separator and no separator fall out
of the same rule (pinned by a boundary test and a randomised oracle). The separator is
`self.separator as u8`, the low byte of the char, exactly as before (`é` and `→` are tested and
behave as they did).

**What it removed.** The fold no longer builds `(offset, len)` segment lists or compares bytes: it
keeps one `seps: Vec<u16>` (positions of the separators of the last key), truncates it to `shared`,
and scans only `name[lcp..]` for new separators, so the shared prefix's bytes are never read.
Group rows are built from the separator positions.

**Two further changes the profile asked for** (both in the same loop):
- *Descendant counts without the per-key pass.* A group records `placed` when it opens and is given
  `placed_at_close - placed_at_open` when it closes (when a later key shares fewer segments, or at
  the end). The `counts` column (8 MB at 1M, one push per row) and the per-key
  `for g in open_groups { counts[g] += 1 }` are gone; the fix-up pass only writes the inverse now.
- *No per-separator hashing for the collapsed test.* The old per-key `ancestor_collapsed(name)`
  probed the hash set once per separator (the 115 ms collapsed-prefix cost in the profile). With no
  irregular prefix, the key is hidden exactly when the innermost open group is collapsed (the groups
  stop at the first collapsed prefix), so that is read directly. And `prefix_is_collapsed`
  (still needed for each *new* group) is skipped unless a collapsed prefix has that byte length
  (`collapsed_lens`, recomputed by `toggle`). The irregular-prefix path (a hand-toggled prefix that
  does not end in the separator) keeps the old linear scan.

**Where the LCP lives: a parallel column on `KeyView` (`lcp: Vec<u32>`), not carried through
`FoldProgress`.** Reasons: (1) fold-only jobs (collapse, expand, entering tree mode over a Name
view) have no Sort stage, so a job-carried LCP could never reach the hottest everyday case, while a
column on the view is already there when they start; (2) one source for the sync path and the job
path (the job moves its sorter's by-product into the view at the swap, `KeyView::install`, as it
does with `order`); (3) the fold takes `Option<&[u32]>`, so the same function is used by both.
Cost: 4 MB at 1M (u32; u16 would be exact since names are capped at 65535 bytes, but the job's
sorter emits u32 and a conversion pass costs more than the 2 MB it would save).

**Validity.** `KeyView::name_lcp()` returns the column only when the order was built for Name and
the column is exactly as long as `order`. It is set by `rebuild` and `install`, **patched** by
`narrow` (a kept row's new LCP is the minimum of the adjacent LCPs since the previous kept row,
exact because the LCP of two names in sorted order is the minimum of the adjacent ones), and cleared
by `extend` (Scan order only). Every other sort leaves it empty. **Fallback:** with no column the
fold computes each key's LCP against the previous key's name itself (`common_prefix`, 8 bytes at a
time) and runs the same code, so there is one fold, not two.

**Tests.** `the_lcp_fold_equals_the_legacy_fold_for_every_separator_and_collapsed_set` keeps the old
fold verbatim as an oracle (`legacy_rows`) and compares rows (so descendants, expanded flags, depth)
and the inverse for 400 random keyspaces over five separators (`:`, `/`, `.`, `é`, `→`), empty
segments, trailing separators, bare names, prefix-of-neighbour names, invalid UTF-8 and random
collapsed sets including irregular prefixes, with the view's LCP, with none, and sliced. Plus the
`cache:` / `cache:user:` regression, the boundary cases, narrowing keeps the column equal to a brute
force LCP, `extend`/other sorts carry none, and the job equivalence suite additionally asserts the
swapped view's column. A mutation (`<` to `<=`) fails two of them. Task 2 and 3 tests pass unchanged.

First measurement (local, load avg 6.5, noisy): fold alone 207 -> 98 ms; collapse time-to-swap
183 -> 59 ms; tree toggle time-to-swap 309 -> 181 ms; toggle from Name-sorted flat 99 ms to swap,
worst step 4.0 ms.

### Phase 3: hiding the per-key miss

Measured at 1M random_deep, Name-sorted view, minimum or median of 5-15 runs, load average 2.6-4.
(One trap worth recording: a first matrix of these runs was invalid because zsh does not word-split
`$cfg`, so every run used the default; the numbers below are from the re-run.)

Steps taken before the miss work, each from a profile of the loop: the key pass after phase 2 was
~88 ms. The SWAR separator search (`push_separators`: 8 bytes per word, exact zero-byte test, one
overlapping last word, byte loop under 8 bytes) took it to ~80; keeping each open group's collapsed
flag on the open-group stack instead of reading `rows[r]` (a row up to 47 KB back, per key) took it to
~69 ms. Both are plain safe Rust.

| Variant | Key pass / fold alone |
|---|---|
| none | 69-70 ms |
| (b) software lookahead: touch the name k ahead inside the loop, k = 4 / 8 / 16 / 32 / 64 | 79 / 80 / 81 / 78 / 78 ms (worse: the touch is an ordinary load that blocks retirement, and the loop body is too long to overlap it) |
| chunk touch pass (loads only, over the next 64 / 256 / 1024 / 4096 / 16384 keys, then the fold loop) | 29.6 / **28.5** / 29.8 / 30.5 / 35.3 ms |
| (a) gather: copy each chunk's name tails into one buffer, then scan the buffer (isolated harness: name read + shared + tail scan only) | none 35.0 ms, chunk touch 17.9 ms, **gather 37.0 ms** (chunks of 512 and 4096) |
| floor: the same fold over an arena stored in name order (no misses) | 16 ms |

Kept: **the chunk touch pass**, a degenerate gather that copies nothing (`touch_names`, chunk of
256 keys, loads the line at the first byte past the shared prefix and the line with the last byte).
It is a tight loop of independent loads the core overlaps dozens at a time; the fold loop then reads
its names from L1/L2. The gather with a copy only added the copy to the same misses, and the in-loop
lookahead loses. No `unsafe`, no arch intrinsics. The fold lands 12 ms above its no-miss floor.

After phase 3 (local): fold alone 207 -> 27 ms (target 60); collapse time-to-swap 183 -> 19 ms;
tree toggle time-to-swap 309 -> 108 ms (target 250); toggle from a Name-sorted flat view 225 -> 30 ms;
worst fold step at slice 32768: 8 ms -> 1.05 ms.
