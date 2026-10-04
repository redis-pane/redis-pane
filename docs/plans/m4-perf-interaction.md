# M4 task 4: Million-key interaction

Status: **done.**

## Resolved decisions (build-time)

1. **Filter narrowing (decision 1) landed, with a correction: glob is only
   conditionally safe, fuzzy is always safe.** `KeyView` records what its
   `order` was last built for (`Applied`: filter, mode, sort, and how many
   keys it covered), and `KeyView::can_narrow`/`narrow`
   (`crates/core/src/state/view.rs`) re-filter only the rows already shown.
   - *Fuzzy* is a subsequence test, so appending a character can only remove
     matches.
   - *Glob* is **not** safe for every extension, contrary to the plan's
     framing. `*a` matches `xa`; its extension `*ab` matches `xab`, which `*a`
     does not. A pattern with no wildcard is a substring search (extension
     keeps the old text as a substring: safe) and a pattern ending in `*`
     absorbs what follows (safe), but a wildcard pattern ending in a literal
     is anchored and must fall back to a full rebuild. Both claims are proven
     exhaustively in tests (every pattern pair to length 4 against every name
     to length 5), plus a test pinning the counter-example.
   - *Extra preconditions the plan did not list:* the sort must be `Scan` or
     `Name` (a rebuild re-sorts by metadata that arrived since, so lazy-sort
     orders can differ); the mode and sort must match what the order was built
     under; and the order must cover every loaded key (a key that arrived
     since the last rebuild, as in tree or non-Scan modes mid-scan, is in no
     row to keep). Anything else takes the full-rebuild path.
   - Narrowing is checked against the *applied* filter, never the typed text,
     so an extension typed while a rebuild is pending is never narrowed from a
     stale order. `KeyView::extend` also filters new keys by the applied
     filter, for the same reason. Tree mode still runs `Tree::rebuild` after
     narrowing; selection and Open key relocation are unchanged.
2. **Debounce (decision 2): a shell timer, 100 ms, trailing-edge.** The typed
   text lands in the filter box on every keystroke. A keystroke that cannot
   narrow sets `State::filter_pending` and returns `Command::ScheduleFilterRebuild`;
   the shell (`Debounce` in `crates/app/src/terminal.rs`) restarts a 100 ms
   deadline each time and answers with `Msg::FilterRebuildDue`, which the core
   honours only if `filter_pending` is still set. Every rebuild, whoever runs
   it, clears the flag, so a late timer is a no-op. `Esc` clears immediately
   (it does not wait), `Enter` flushes a pending rebuild, and nothing needs
   handling on quit. 100 ms is above the ~30 ms auto-repeat of a held
   Backspace and the 50-80 ms gap of a fast typist, and below the ~150 ms where
   a list that lags the box starts to read as lag — a judgement, not a
   measurement. Known gap: a mouse click on a row inside that window acts on a
   not-yet-filtered row (still a real key).
3. **`Tree::rebuild` allocates nothing per key (decision 3), by restructuring
   the loop, not by a dependency.** Two reusable segment buffers are swapped
   per key; the prefix through a segment is a slice of the key's own name;
   no `smallvec`. A prefix with valid UTF-8 probes the collapsed set with a
   borrowed `&str`; only invalid bytes pay a lossy conversion, which keeps a
   collapsed `\u{FFFD}:` group matching exactly as before.
4. **Collapsed lookup is O(1) (decision 4): `HashSet<String>`, not arena
   `(offset, len)`.** An arena offset names one key's copy of the text, so
   every probe would still need a byte comparison, and the set is a handful of
   entries. `toggle`/`is_collapsed` keep their `&str` signatures.
   `ancestor_collapsed` probes once per separator in the name. An `irregular`
   count tracks collapsed prefixes not ending in the separator (nothing in the
   app produces one, but `toggle` accepts any `&str`); while any exists the old
   linear `starts_with` scan is used, so behaviour is identical.
5. **`row_of` uses an inverse index (decision 5), on `KeyView` and on `Tree`,
   not on `State`.** Each is the inverse of a structure rebuilt or extended in
   exactly one place (`KeyView::rebuild`/`extend`/`narrow`, `Tree::rebuild`),
   so no caller can change one and forget the other. Both are dense
   `Vec<u32>` over Loaded set indices (`u32::MAX` = no row), counted in
   `heap_bytes`; `State::row_of` picks by `tree_mode`. It costs ~4 MB each at
   1M keys. It makes lookups O(1) *between* rebuilds; it does not make a
   rebuild cheaper (the fill is O(n)), which is why `rebuild_list` itself
   gained nothing from it.
6. **Metadata fetches (decision 6): a core-minted epoch decides staleness; a
   shell-side ledger does dedup.**
   - *The build-time question — can a late stale reply cause visible
     incorrectness today?* **Yes, in one case.** A reply carries bare Loaded
     set indices. A rescan (`scan_started` -> `LoadedSet::clear`, refill in a
     different `SCAN` order) renumbers them, so a reply that outlives the
     rescan writes a type, TTL and size — or a *tombstone* — onto an unrelated
     key. Within one numbering a late reply is correct and merely redundant,
     so discarding it for scroll-away would only cause a re-fetch; it is
     applied.
   - *A shell-side counter does not suffice (corrected after review).* The
     first cut bumped a shell generation when the shell handled
     `Command::StartScan`. But the core renumbers only when `Msg::ScanStarted`
     is dequeued, at least one `SCAN` round trip later; in between it still
     shows the old keys and can emit `FetchMetadata` (scroll, resize) with old
     indices, which the shell had already called "current" — its reply then
     passed the shell's check and was applied after `ScanStarted`. A second,
     narrower window: a task could pass its own check and reach `tx.send` after
     the loop had already processed `ScanStarted`, since the check ran in the
     task, not at dequeue. A timing argument cannot close either.
   - *The fix is the plan's "core-checked token", shaped like `InfoToken`:*
     `MetadataEpoch` (`crates/core/src/command.rs`), minted only by the core.
     `State::metadata_epoch` is bumped exactly where indices are renumbered —
     `scan_started`'s `keys.clear()`, the only place keys are cleared.
     `Command::FetchMetadata { indices, epoch }` carries it; the shell echoes
     it unchanged in `Msg::MetadataBatch { epoch, .. }`; `metadata_batch` drops
     a batch whose epoch is not current, before touching anything. Staleness
     is now a fact about the message, decided in the core at dequeue.
   - *The shell ledger* (`crates/app/src/metadata.rs`) follows the epoch it
     sees: a `FetchMetadata` under a new epoch forgets what is in flight and cancels
     the old epoch's fetches. That is only an optimization (no wasted round
     trips, no dedup entry held by a fetch whose answer will be dropped);
     correctness does not depend on it or on the task-side `finish()` check.
     The ledger also resets on **reconnect**, which is still needed for a
     different reason: fetches issued on the replaced client may never be
     answered, and an index held in flight by one that hangs would never be
     asked for again.
   - Dedup: `claim` skips indices with a fetch in flight. A `Claim` dropped
     without `finish` — failure, cancellation, panic — releases its indices, so
     none can wedge. A failure from the current generation still surfaces as
     `Msg::Failed` exactly as before; only a superseded one is dropped.
   - **Reversed from the plan, by the user: a row reported gone is
     re-requested when it re-enters the window.** The plan said a tombstoned
     row should be remembered as answered and never re-requested. The first
     cut did that, which meant a key deleted and then recreated kept its
     `✕ gone` badge in the list until a rescan, reconnect or its own Viewer
     read — a stale display, against the product's no-stale-display
     principle (ADR-0006's spirit). So the ledger keeps no gone set. The cost
     is at most one window of extra `TYPE`s riding the same pipelined batch,
     never an extra round trip; in-flight dedup still stops overlapping
     windows from re-asking while a fetch is outstanding.

## Measured (release, local, `--test-threads=1`)

| Path at 1M keys | Main (before task 4) | After | Ceiling now |
|---|---|---|---|
| Filter keystroke (extends the query) | 22.3 ms | **0.27 ms** | AT TARGET 16 ms |
| Filter first character (from empty) | n/a | 3.9 ms | AT TARGET 16 ms |
| Filter backspace (rebuild deferred) | n/a | 0.04 ms | AT TARGET 16 ms |
| Filter full rebuild (after debounce) | n/a (every keystroke) | 21.6 ms (CI 32.4 ms) | CEILING 49 ms |
| Tree toggle | 126 ms | 44.6 ms (CI 57.4 ms) | CEILING 89 ms |
| Sort change | 3.76 ms | 4.2 ms | AT TARGET 16 ms |
| Whole-scan fold, tree mode | 0.60 s | 0.24 s | gate 10 s |
| Whole-scan worst page | 105 ms | 32 ms (CI 45.2 ms) | CEILING 68 ms |
| ScanBatch page, tree mode | 0.93 ms | ~1.0-1.2 ms | AT TARGET 16 ms |
| LoadedSet + View + Tree | 75.8 MB | 83.4 MB | AT TARGET 125 MB |

Ceilings still above the PRD target are 1.5x the CI measurement, rounded up —
task 2's rule — since CI's runner is the slower machine (~1.3-1.5x local).
Calibrated on PR #60's CI run; first set at 2x local, which left the
debounced rebuild only 1.36x headroom on CI.

## Context (as planned)

PLAN.md M4 row 4: "Million-key interaction · Proves: a filter keystroke at 1M keys meets the 16ms
frame budget: it narrows from the previous result when the query extends, and otherwise rebuilds
once, debounced. The tree rebuild allocates no per-key `String` or `Vec`, and collapsed-group
lookup is O(1). `row_of` uses an inverse index. Metadata fetches are deduplicated and the stale
ones cancelled. Budgets pass."

Where task 3 (`m4-perf-scan.md`) fixes the cost of *arriving* keys, this task fixes the cost of
*acting on* a keyspace that has already arrived — the hot paths a reader actually drives by hand:

- **Every filter keystroke rebuilds from scratch.** `crates/core/src/update/keys.rs`'s
  `filter_key`/`filter_key_keys_pane` call `state.rebuild_list()` on effectively every character
  typed or deleted (multiple call sites in that file). `rebuild_list` → `View::rebuild`
  (`crates/core/src/state/view.rs`) always does `self.order.clear()` followed by a full `0..keys.len()`
  scan re-matching every key against the new filter — at 1M keys, every keystroke re-examines the
  whole keyspace, with no reuse of the previous keystroke's narrower result even when the new query
  is a strict extension of the old one (typing one more character into an already-matching filter).
- **The tree allocates per key, per rebuild.** `Tree::rebuild` (`crates/core/src/state/tree.rs`)
  calls `split_segments`, which allocates a fresh `Vec<(u32, u16)>` for every key's segments, and
  builds `prefix` as a `String` via repeated `String::from_utf8_lossy(...)` + `push_str` per
  segment, per key — two allocations (at least) per key on every rebuild, and `rebuild_list` can
  trigger a tree rebuild from a single filter keystroke.
- **Collapsed-group lookup is linear.** `Tree::is_collapsed` and `Tree::ancestor_collapsed` are
  both `self.collapsed.iter().any(|p| ...)` — a linear scan of every collapsed prefix, called once
  per key, per segment depth, during every `Tree::rebuild`. With many collapsed groups (plausible
  on a deep, consistently-prefixed million-key keyspace) this compounds the per-key cost further.
- **`row_of` is a linear scan**, confirmed in `m4-perf-scan.md`'s reading of
  `crates/core/src/state/mod.rs`: `(0..self.row_count()).find(|&row| self.key_at(row) == Some(index))`.
  Called from `relocate_open_key` on every `rebuild_list`, so every filter keystroke, sort change
  or tree toggle pays an O(row_count) scan on top of the rebuild itself.
- **Metadata fetches have no dedup or cancel.** `crates/app/src/terminal.rs`'s `fetch_metadata`
  resolves the visible window's indices, then `tokio::spawn`s one task per call with no stored
  `JoinHandle` and no tracking of which indices already have a fetch in flight — confirmed by
  reading the method: it spawns and returns, nothing is kept to cancel or dedup against. Scrolling
  quickly re-triggers `Command::FetchMetadata` for overlapping windows repeatedly, and a tombstoned
  (deleted/expired/evicted, `"none"` `TYPE`) row that re-enters the visible window — e.g. the reader
  scrolls away and back — is re-requested every time, since nothing remembers it already answered
  "gone."

## Decisions

1. **Filter narrowing: reuse the previous result when the query extends.** When the new filter
   string starts with the previous one (glob or fuzzy — **confirm at build time** whether fuzzy
   narrowing is safe to reuse this way; fuzzy matching is not necessarily monotonic in the same
   sense glob prefix-narrowing is, since a fuzzy match is about subsequence membership, not a
   strict substring relationship — it is plausible narrowing-from-previous-result holds for fuzzy
   too since adding a character to a fuzzy query can only remove matches, never add them, but this
   should be proven with a unit test before relying on it, not assumed from this doc), `View`
   re-filters only the indices already in `self.order` rather than the whole `LoadedSet`. When the
   query is *not* an extension (backspace past where narrowing started, a pasted replacement, or
   the very first character), it falls back to a full rebuild — but debounced (decision 2), not
   once per keystroke.
2. **A full rebuild, when needed, is debounced rather than run per keystroke.** Typing several
   characters quickly should not trigger several full million-key rebuilds; a short debounce
   (shell-side timer, since the core owns no clock — **confirm at build time** the exact window,
   something on the order of 50–150ms that is below "feels laggy" but above typical inter-keystroke
   timing) coalesces a burst of keystrokes into one rebuild. This is a shell concern layered on top
   of the core's pure filter logic, the same way the Dashboard's poll interval and (per
   `m4-perf-scan.md`) the redraw cadence are shell-owned timers feeding `Msg`s into the core.
3. **The tree rebuild allocates no per-key `String` or `Vec`.** `split_segments` should return
   segment boundaries without a heap `Vec` — e.g. written into a reusable `SmallVec`-style
   fixed/stack buffer, or computed inline within `Tree::rebuild`'s existing loop without a separate
   allocating function — and `prefix` comparison/`is_collapsed` lookups should work against arena
   byte slices directly rather than building a `String` via `from_utf8_lossy` per segment.
   **Confirm at build time** whether this needs a small vendored/hand-rolled fixed-capacity vector
   (consistent with CLAUDE.md's "no runtime dependency" preference for a single static binary — a
   `smallvec`-crate dependency is a reasonable, common choice but should be a deliberate addition,
   not a reflex) or whether `Tree::rebuild`'s loop can be restructured to avoid materializing
   segments as a standalone collection at all (iterating separator positions directly).
4. **Collapsed-group lookup is O(1).** `Tree.collapsed: Vec<String>` becomes (or is joined by) a
   hash-based lookup — `HashSet<String>` or `HashSet<(u32,u16)>` keyed by arena offset/len if that
   avoids a second String allocation per collapsed prefix. **Confirm at build time** which keying
   is cheaper given how `toggle`/`is_collapsed` are called elsewhere (both currently take a `&str`
   prefix built by the caller, which may itself need revisiting once the per-key `String` building
   in decision 3 is removed).
5. **`row_of` uses an inverse index.** A `Vec<u32>` (or `HashMap<usize, usize>`, but a dense
   `Vec` indexed by Loaded-set index mapping to current row is more consistent with this codebase's
   "parallel arrays, not a `Vec` of structs" discipline, ADR-0010) maintained alongside
   `View.order`, updated wherever `order` is built or extended (including task 3's incremental
   append path) so `relocate_open_key` becomes an O(1) lookup instead of an O(row_count) scan.
   **Confirm at build time** whether this inverse index lives on `View` (natural, since it is
   `order`'s inverse) or on `State` alongside `row_of` itself.
6. **Metadata fetches are deduplicated and the stale ones cancelled.** The shell should track which
   indices currently have an in-flight fetch (e.g. a `HashSet<usize>` on the `Shell`/terminal-loop
   struct) and skip re-requesting an index already in flight; when the visible window changes
   before an old fetch's reply lands, the old fetch's result for indices no longer in any live
   window should be discarded rather than applied (a token-per-request scheme, similar in spirit to
   the Dashboard's core-minted token from `m3-dashboard.md`, but this one can be shell-side only
   since metadata replies are idempotent writes into `LoadedSet` and do not need the core to
   arbitrate staleness the way a Dashboard poll-vs-manual-refresh race does — **confirm at build
   time** whether a shell-side generation counter is sufficient or whether this needs a `Msg`-level
   token the core checks, by looking at whether a stale metadata reply landing late can currently
   cause any visible incorrectness beyond wasted work). A tombstoned row (`gone`, from
   `metadata_batch`'s existing handling) should be remembered as answered and never re-requested
   (**reversed at build time by the user** — see Resolved decisions, 6)
   just because it re-enters the visible window.

## Architecture

Decisions 1, 3, 4 and 5 are core-side, pure state-transition changes (`View`, `Tree`,
`State::row_of`) — no I/O, straightforward golden-frame and unit-test coverage. Decision 2 (the
keystroke debounce) is shell-side, the same "clock lives in the shell, core only reacts to a
`Msg`" split as task 3's redraw coalescing — the core should not need to know a debounce happened;
it just receives fewer, coalesced filter-change `Msg`s, or one `Msg` that the shell only emits
after the debounce window closes. Decision 6 is entirely shell-side (`crates/app/src/terminal.rs`'s
`fetch_metadata` and whatever dispatches `Command::FetchMetadata`); no new core `Msg`/`Command`
shape should be needed since `Msg::MetadataBatch` and `gone` handling already exist — this is
about *when* the shell issues the fetch, not what the core does with the reply.

## Files touched

| File | Change |
|---|---|
| `crates/core/src/state/view.rs` | narrowing-from-previous-result filter path; inverse index for `row_of` |
| `crates/core/src/state/mod.rs` | `row_of` reads the inverse index instead of scanning |
| `crates/core/src/state/tree.rs` | `split_segments` / `Tree::rebuild` avoid per-key `String`/`Vec`; `collapsed` becomes O(1)-lookup-backed |
| `crates/core/src/update/keys.rs` | filter keystroke path feeds the narrowing logic instead of always calling full `rebuild_list` |
| `crates/app/src/terminal.rs` | debounce timer for filter-triggered full rebuilds; `fetch_metadata` dedup/cancel bookkeeping, tombstone-aware skip |

## Testing

- **Core unit tests**: narrowing a glob/fuzzy filter from a previous result produces the same final
  `order` as a full rebuild from scratch (equivalence test, run across several filter sequences
  including non-extending ones that must fall back correctly); `Tree::rebuild` produces identical
  `rows`/`descendants` output before and after the allocation changes (golden-equivalence, not just
  a perf claim); `is_collapsed`/`ancestor_collapsed` return the same answers under the new lookup
  as the old linear scan, across the existing tree test fixtures in `crates/core/src/state/tree.rs`'s
  own test module; `row_of` via the inverse index matches the old linear-scan answer for every
  index across a representative `State`, including after task 3's incremental scan-append path.
- **Release-mode perf tests** (harness from `m4-perf-harness.md`): a filter keystroke extending the
  previous query, at 1M keys, completes within the 16ms budget; a non-extending filter change
  (debounced) completes within budget once the debounce window elapses; a tree-mode toggle at 1M
  keys with many collapsed groups stays within budget.
- **Golden frames**: unaffected in content (same rule as task 3) — existing filter/tree golden
  frames must render pixel-identical output after these changes.
- **A dedicated dedup/cancel test** (app-level, no Docker needed if it can be driven with a fake
  client — **confirm at build time** whether `fetch_metadata`'s existing test coverage, if any, can
  be extended, or whether this needs a new harness): rapidly overlapping `Command::FetchMetadata`
  calls for the same indices spawn at most one in-flight fetch per index; a tombstoned index is not
  re-requested on a subsequent call for a window that includes it.

## CLAUDE.md rules this binds

- **The key list is columnar and capped** (ADR-0010) — the inverse index and the tree's allocation
  changes must stay in the same "parallel arrays, not a `Vec` of per-key structs" shape; a
  `HashMap<usize, Vec<u8>>`-style per-key cache would violate the discipline this task is
  specifically trying to uphold at scale, not just avoid for its own sake.
- **Lists are virtualized. Render cost is a function of viewport size, not keyspace size** — this
  task's metadata-fetch dedup/cancel work is the direct enforcement of that rule for the one path
  (`fetch_metadata`) that currently has no guard against re-fetching work outside the viewport's
  actual needs.
- **A keystroke must be answerable in one frame (16ms) regardless of what the network is doing** —
  the whole point of this task; filter narrowing and debouncing are how that holds at 1M keys where
  it does not today.

## Out of scope

- **Changing the filter grammar itself** (glob/fuzzy semantics) — this task optimizes evaluation,
  not matching behavior; `crates/core/src/state/view.rs`'s `glob`/`fuzzy` functions' actual matching
  rules are untouched.
- **The scan-arrival cost path** — `m4-perf-scan.md`'s task, not this one, though this task's
  inverse-index and `row_of` work is reused by that task's incremental-append path (see that doc's
  decision 3).
- **A general LRU or smarter prefetch strategy for metadata** — dedup and cancel-the-stale are the
  asked-for behavior; a prefetch-ahead-of-scroll optimization is a real possible future
  improvement but not what PLAN's row asks for here.
