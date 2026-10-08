# M2 remaining: rename, copy, member rename, bulk delete

Status: **tasks 11-14 done** (task 5 parked). Planned 2026-10-08, after M6 shipped as `0.1.0-beta.3`. Each
task gets its own plan doc and its own Sonnet subagent run, and is merged before the next starts.
This is the same process as M4–M6.

## Context

M2 (PLAN §5) shipped tasks 1–4 and 6–10: the mutation chokepoint, delete, value edits for every
type, and TTL. Still unbuilt:
- **R4.3**: rename, copy and bulk operations.
- **Renaming a single item inside a key.** Tasks 6, 7 and 9 deferred this to task 14: a Hash
  field's name, a Set member and a ZSet member.
- **Move-across-db.** PLAN's parked note still stands.

M5 added Cluster rules for tasks 11–13 to PLAN §5.

## Decisions (user, 2026-10-08)

- **Scope is tasks 11–14.** Task 5, the `$EDITOR` escape hatch, stays parked, and PLAN §5 says so.
- **Keys:**

  | Key | Where | Action |
  |---|---|---|
  | `R` | keys pane | rename the key under the cursor |
  | `D` | keys pane | duplicate it under a new name (`COPY`) |
  | `R` | value pane | rename the field or member under the cursor |
  | `Space` | keys pane | toggle multi-select, as DESIGN §4 reserves |

  Every binding is scoped to its pane, per the growth rule (DESIGN §4, ADR-0020). Duplicate avoids
  `c`, which already means the clipboard.
- **Process:** I merge each PR once it is green and start the next. I stop only on a failure or a
  design question. No release tag is pushed without the user.

## Shared rules, binding on every task

- **Every write goes through the chokepoint.** The flow is the existing `PendingMutation` stage →
  preview → `y` → execute path. Read-only Mode refuses at confirm, after the real command is
  composed (R4.4, R4.5, DESIGN §6.5).
- **Collisions are refused before anything is written,** never surfaced afterwards as a server
  error. Each write is atomic in Redis:
  - `RENAMENX` for rename;
  - `COPY` without `REPLACE`;
  - `RESTORE` without `REPLACE`, which refuses with `BUSYKEY`;
  - `HSETNX` / `SADD` / `ZADD NX` inside a guarded script for member rename.

  The preview also checks ahead and warns. Overwriting an existing target is **out of scope**,
  because it is the destructive variant: it needs its own decision, ADR and friction rule.
- **No key ever appears outside the scan.** A renamed or duplicated key reaches the Loaded set
  through the same path as a scanned one, so `scan_batch` stays the only place the cap is
  enforced (CLAUDE.md). The old name of a renamed key is marked gone, using the badge machinery
  delete already uses.
- **On a Cluster** (ADR-0022, PLAN §5):
  - **Rename** across slots is refused at the preview, which explains that `RENAME` fails with
    `CROSSSLOT` there.
  - **Copy** across slots falls back to `DUMP` + `RESTORE` (no `REPLACE`, the TTL carried in
    `RESTORE`'s argument), behind the same confirmation.
  - **Bulk delete** is grouped per owner.
- **Errors surface as notifications that name the failing command** (R7.4).

## Task table

| # | Task (plan doc) | Proves |
|---|---|---|
| 11 | Rename ([`m2-task11-rename.md`](m2-task11-rename.md)) | `R` → a name capture prefilled with the current name → preview `RENAMENX old new`. A taken target is refused at preview and again atomically at execute. The Open key and the selection follow the new name. Cross-slot is refused on a Cluster |
| 12 | Copy ([`m2-task12-copy.md`](m2-task12-copy.md)) | `D` → the same capture → preview `COPY src dst`, and the TTL carries over. A taken target is refused. On a Cluster across slots: `DUMP` + `RESTORE ttl`, refused with `BUSYKEY` if the target is taken |
| 14 | Member rename ([`m2-task14-member-rename.md`](m2-task14-member-rename.md)) | `R` in the value pane renames a Hash field (value kept), a Set member, or a ZSet member (score kept), each as one guarded script. It refuses when the new name is taken or the old one is gone, and never recreates a gone key |
| 13 | Bulk delete ([`m2-task13-bulk-delete.md`](m2-task13-bulk-delete.md)) | `Space` marks keys. `d` with marks stages one bulk delete listing the count and the keys. `prod` requires typing the count; elsewhere one `y`. Deletes are batched and per key, so they are Cluster-safe. Single-key delete is untouched |

**Order: 11 → 12 → 14 → 13.** Copy reuses rename's capture and collision machinery. Member rename
is independent. Bulk delete is the largest and touches the key list's state, so it goes last.

## How each task runs

Same as M5/M6:
- start from synced `main` on an `m2-<name>` branch;
- one Sonnet subagent working phase by phase, pushing after each phase;
- before the PR: fmt, clippy `-D warnings`, `cargo test --workspace`, the boundary check, the
  Docker suite (foreground, bounded);
- golden frames for every new prompt, dialog and hint bar;
- an `## Outcome` section in the plan doc;
- I merge when CI is green.

Close-out after task 13 updates PLAN §5 (rows done, task 5 still parked), PRD R4.3, DESIGN §4/§6.5,
and README/ALPHA. A version bump and release happen only with the user's go-ahead.
