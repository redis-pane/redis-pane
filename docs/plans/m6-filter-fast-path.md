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
