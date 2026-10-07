# M6 task 6: Filter fast path (gated)

Status: **planned, gated.** Build this only if, after tasks 3–5, task 1's harness shows the
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
