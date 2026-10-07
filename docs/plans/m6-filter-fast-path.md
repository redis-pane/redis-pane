# M6 task 6: Filter fast path (gated)

Status: **done** (PR #83; see the Outcome). Gate was met: build this only if, after tasks 3–5, task 1's harness shows the
filter pass is the longest stage left in a filter rebuild at 1M keys, deep names, random order.
Otherwise mark it **not needed**, give the numbers, and skip to task 7.

## Context

[`m6-planning.md`](m6-planning.md), row 6. A filter rebuild offers every key to `matches`, which
calls `glob_at` for each key. That takes 24–41 ms at 1M keys. It already reads the arena in
index order, so it is sequential and needs no access-pattern fix. Once task 3 slices it, it no
longer costs a frame. It only adds to how long `rebuilding N%` stays on screen.

## Decisions (if built)

1. **The fast path covers glob patterns with no `*` or `?`,** which `matches` already treats as
   a substring search. That is the common case. Wildcard globs and fuzzy patterns keep today's
   per-key matching.
2. **Case-insensitive, as today.** `glob_at` compares with `eq_ci`. The search scans the arena
   for the pattern's first byte in both cases, then verifies the match case-insensitively. It
   needs no lowercase copy of the arena, which would cost another 40 MB.
3. **Hits are mapped back to keys, then validated.**
   - Each hit's offset is mapped to its key by binary search over `offsets`, or by advancing a
     key cursor as hits arrive in order.
   - A hit that spans two adjacent names doesn't count.
   - A key is reported once, however many hits it has.
4. **It slices like the filter stage it replaces:** a key range per step, in task 3's terms.

## Testing

- **Equivalence with `matches()`:** for random arenas and patterns, including mixed case, a
  match at a key boundary, a key matching several times, and non-ASCII bytes, the fast path
  returns exactly the keys `matches()` would.
- **Perf:** the filter stage at 1M `random_deep`, and the time until the new list appears for a
  filter rebuild.

## Phase log

### Phase 1 - baseline on main (load average 9 during the run, so figures are upper-ish)

1M keys, release, local:

| Measure | sorted | random_deep |
|---|---|---|
| filter first character (sync narrow) | 4.2 ms | 21.7 ms |
| filter keystroke (narrow from `user:0000`) | 0.27 ms | 0.56 ms |
| filter backspace (rebuild deferred) | 40 us | 47 us |
| debounced rebuild, trigger to swap | 21.9 ms | 40.2 ms |
| of which filter stage | 21.7 ms | 40.0 ms |

Profile: the first-character case is compute-bound, not cache-bound. Both fixtures walk the
arena in index order (the order is scan order), so access is sequential either way; what
differs is the name. Sorted names start `user:`, so `windows(1).any(eq_ignore_ascii_case)`
exits at byte 0; random_deep names start `app:N:tenant:N:user:` and the scan runs ~20 bytes per
key, a scalar compare per window. The cost is the per-window scan, which a vectorised
first-byte search replaces. (In tree mode the order is name order and `narrow` is also random
access; that is why narrow gets an index-order search plus a mark, phase 3.)

### Phase 2 - the searcher

`state/search.rs`: `Substring` (`memchr2` for the first byte in both cases, ASCII-case
verification, a key cursor over `offsets`) and `filter_range` (the dispatch every caller
shares). `memchr` is added as a direct dependency of core: it was already in core's tree via
`serde_json`, same version, no new crate. Equivalence property tests against `matches()`.

### Phase 3 - wiring

`filter_range` serves the job's Filter stage (a key range per step, so the searcher resumes at
key boundaries: decision 4), `KeyView::rebuild`, and `KeyView::extend` (all one call, so it
was trivially shared). `narrow` gets the index-order search plus a mark (below). Existing
filter/narrow tests, task 2's refold tests, task 3's job-equivalence tests and task 5's fold
oracle tests pass unchanged. Added: search-vs-`matches` equivalence (whole arena, arbitrary key
ranges, slice-by-slice stepping, named edge cases) and narrow-by-search == narrow-by-`matches`
(rows, order, inverse, LCP column) over random keyspaces, Scan and Name orders.

### Phase 4 - perf and ceilings

Local runs had load 3-9 (noted per run); CI figures are from this PR's two perf runs.

## Outcome

Built as decided (1-4), plus the narrow path. Filter first character at 1M random_deep is now
**AT TARGET** (the harness's label): 6.0 ms local, 9.3-9.5 ms CI against 16 ms.

**Design.**
- `state/search.rs`: `Substring` searches the arena with `memchr2` (both cases of the first
  byte), verifies ASCII-case-insensitively (bytes >= 0x80 never fold, as `eq_ignore_ascii_case`),
  and advances a key cursor over the offsets to attribute hits. A candidate that would run past
  its key's end is skipped to the next key (every later start in that key would overflow too);
  a hit skips to the next key, so each key is reported once; empty names are stepped over. It
  applies only to glob, non-empty, no `*`/`?`: exactly where `matches` does a substring search.
  Wildcards and fuzzy keep the per-key path.
- `memchr` is a new direct dependency of core, but not a new crate: it was already in core's
  tree through `serde_json` (same 2.8.3). No `unsafe`, no I/O; the boundary check is unchanged.
- `narrow`: when the rows to test number at least a quarter of the Loaded set
  (`NARROW_SEARCH_FRACTION`), it searches the whole arena in index order (sequential), sets a
  bit per hit in a `Vec<u64>` of `keys.len()/64` words (125 KB at 1M, allocated per narrow,
  cost not measurable), then runs the unchanged walk over `order` testing the bit. Order, inverse
  patch and the LCP running minimum are the same code, so behaviour is identical; only the
  keep-test changed. Below the fraction the per-row `matches` is cheaper than touching every
  name and is kept. The search covers all keys (a superset of `order`), harmless since the walk
  only reads bits of rows it has.
- The Filter stage slices unchanged: `filter_range(keys, filter, mode, cursor, end, ..)`.

**Profile (phase 1).** The first-character cost was compute, not cache: a scalar
`windows().any(eq_ignore_ascii_case)` per key. The arena is walked in index order (scan order
is index order) either way. random_deep names put the first `u` ~20 bytes in, so the scan is
long; sorted names start `user:` and exit at byte 0 (hence the 4 vs 22 ms gap).

**Before / after, 1M keys.** Before = main at the start of this task (local, load 9) and task
5's recorded CI figures; after = this branch.

| Measure | local before | local after | CI before | CI after |
|---|---|---|---|---|
| filter first character, random_deep | 21.7 ms | 6.0 ms | 20-35 ms | 9.3-9.5 ms |
| filter first character, sorted | 4.2 ms | 4.1 ms | - | 9.3-9.4 ms |
| filter keystroke, random_deep | 0.56 ms | 0.57 ms | - | 2.2-2.4 ms |
| filter backspace (deferred), random_deep | 47 us | 47 us | - | 108-116 us |
| debounced rebuild, time to swap, random_deep | 40.2 ms | 12.9 ms | 36-64 ms | 17.1-18.0 ms |
| ...of which the filter stage | 40.0 ms | 12.7 ms | - | 12.7-15.7 ms |
| ...worst single update | 1.38 ms | 0.43 ms | - | 0.52 ms median |

(The sorted first-character figure did not move: it was never scan-bound. CI sorted shows 9.3 ms
because the CI machine is slower at the frame itself; there was no CI "before" for it on this
branch.) Memory is unchanged (job peak 116.4 MB, after swap 108.8 MB; the narrow bitset is a
transient 125 KB).

**Ceilings.** `filter_first_character_at_1m_keys_random_deep` 52 -> 16 (AT TARGET).
`time_to_new_list_filter_rebuild_random_deep` 88 -> 28 (1.5 x the slowest of two CI runs, 18.05).
All other filter ceilings were already at 16 and untouched.

**Deviations.**
1. The phase 2 commit failed CI on dead-code (the searcher was not wired until phase 3); fixed
   by the phase 3 commit. Expected given the commit-per-phase rule.
2. The filter stage is still the longest stage of a debounced filter rebuild (12.7 ms of 12.9).
   It is now sliced and ~3x shorter; choosing a rarer anchor byte than the first would cut
   candidate verifications for patterns like `user:0000`, not done.
3. Tree-mode (Name order) narrowing is not in the perf harness; it is covered by the
   equivalence test (Name sort) and gets the same sequential search by construction.

**For task 7 (README / ALPHA known limits).** Filtering a 1M-key set takes ~13 ms of CPU
(local) in a debounced rebuild, and the first character narrows in ~6-10 ms; wildcard (`*`,
`?`) and fuzzy filters still test each name individually (~40 ms for a full rebuild at 1M,
sliced, and not measured separately here). Case folding is ASCII-only, by design.
