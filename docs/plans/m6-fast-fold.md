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
