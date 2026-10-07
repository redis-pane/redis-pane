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
