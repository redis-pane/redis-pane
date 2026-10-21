# Packaging, and the road to 1.0

Status: **planned** 2026-10-21. Four tasks, in order. Tasks 1, 3 and 4 run on one Sonnet
subagent each, and each PR merges once every check has finished and passed. Task 2 has steps
only the user can do. Tags are pushed only after the user says so.

## Context

M0–M6 are done, and `0.1.0-beta.6` is out. Nothing is scheduled after M6, and no document says
what ends the beta. The one open product question with a deadline was packaging. PRD §10 said
to decide it "at the end of alpha", and it is still open six betas later.

What ships today is what `cargo-dist` 0.32 (`dist-workspace.toml`) builds: four unsigned
archives (macOS arm64 and x86_64, Linux x86_64 gnu, Windows x86_64), plus shell and PowerShell
installers.

Found while planning:
- **The Linux build fails on the hosts the product is for.** `x86_64-unknown-linux-gnu` is built
  on `ubuntu-22.04` against glibc 2.35. RHEL 8 (glibc 2.28) and Amazon Linux 2 (2.26) refuse it
  with `GLIBC_2.3x not found`, and those are typical bastion images. A static musl build fixes
  that.
- **musl is cheap here.** TLS is `rustls` with `ring` (fred's `enable-rustls-ring`), and nothing
  links OpenSSL, so there are no C-library workarounds.
- **musl's allocator is slow,** and this app holds a million keys in parallel arrays. The perf
  budgets have to be measured on musl, not assumed.

## Decisions (user, 2026-10-21)

- **Channels for 1.0:**
  - musl Linux, `x86_64` and `aarch64`;
  - a Homebrew tap;
  - the shell and PowerShell installers stay.
- **Not for 1.0:**
  - crates.io;
  - gnu arm64 Linux, which musl arm64 covers.
- **No code signing for now,** neither macOS nor Windows. The documented first-run workaround
  stays. Homebrew installs avoid the macOS Gatekeeper prompt for most users anyway. Revisit after
  1.0.
- **The 1.0 gate:**
  1. **Packaging shipped and verified:** the chosen channels are live and installed on clean
     machines.
  2. **No open Severity 1 or 2 work:** nothing open in `docs/UI_TASKS.md` at Severity 1 or 2,
     and no open GitHub issue labelled `bug`.

  Measuring every PRD metric, including the untested "new user finds a key in under 30s", and
  resolving #110/#109 were offered and **not** made gates. They stay open work, not blockers.

## Tasks

| # | What | Who | Delivers |
|---|---|---|---|
| 0 | This plan, ADR-0023, the PRD milestone | Main session | Docs only (below). **Done** in this PR |
| 1 | musl Linux builds | Sonnet subagent | `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl` in the release, verified on old distros, perf measured. Then `0.1.0-beta.7` |
| 2 | Homebrew tap | **User**, then a Sonnet subagent | The user creates the tap repo and its token secret. The subagent adds the `homebrew` installer and publish job, and verifies `brew install`. Then `0.1.0-beta.8` |
| 3 | 1.0 gate check and close-out | Sonnet subagent | Every gate item checked and recorded. Version `1.0.0`, docs drop "beta". Tagged after the user's go |

### Task 0: docs (main session, this PR)

- **ADR-0023, "Distribution channels for 1.0":**
  - **Chosen:** musl Linux (x86_64 and aarch64), a Homebrew tap, and the shell and PowerShell
    installers.
  - **Rejected for 1.0, each with its reason:**
    - signing (cost and paperwork, and Homebrew covers most macOS users);
    - crates.io (adds a second release surface, and cargo users can use the shell installer);
    - gnu arm64 (musl covers it);
    - distro packages (apt, rpm) and Scoop/winget.
- **PRD §9:** a new **"1.0: Release"** milestone with the two-item gate above.
- **PRD §10:** resolve the packaging question, pointing at ADR-0023.
- **PLAN:** a §7.5 that points here.

### Task 1: musl Linux

- **`dist-workspace.toml`:** add `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`.
  - **Keep `x86_64-unknown-linux-gnu`.** The shell installer prefers gnu where the host's glibc
    is new enough and falls back to musl otherwise. Verify that `dist` 0.32 really does this
    (read the generated installer). If it doesn't, ship musl only for Linux and say so.
  - Run `dist generate` so the workflow matches. Never hand-edit `release.yml` beyond what `dist`
    writes.
  - **aarch64 runner:** prefer a native `ubuntu-24.04-arm` runner if `dist` supports choosing it.
    Otherwise use `dist`'s cross toolchain (`cargo-zigbuild` or `cross`). `ring` needs a C
    compiler for the target, so check that it builds.
- **Allocator.** Run the release perf suite (`crates/core/tests/perf.rs`) built for
  `x86_64-unknown-linux-musl`, locally in a Docker container, against the gnu figures.
  - If any budget or ceiling fails, or a figure is more than 1.5× the gnu one, add `mimalloc` as
    the global allocator **for musl targets only**
    (`#[cfg(target_env = "musl")]`, in the app crate, never the core), then re-measure.
  - Record the numbers in this plan.
  - The binary must stay under the 20MB budget. The `binary size` CI job checks the dist
    profile; check the musl artifact by hand.
- **Verify on clean, old machines** (Docker, local):
  - `amazonlinux:2`, `rockylinux:8`, `debian:11` and `alpine:3`, for both x86_64 and arm64 where
    the image exists.
  - On each, the shell installer installs, `redis-pane --version` runs, and
    `redis-pane --probe` reaches a Redis container. Include one TLS target, so `rustls` finds the
    system's CA bundle on a static build.
  - Report a table.
- **Docs:**
  - the installation page and the README: which Linux builds exist, and that they run on old
    distros;
  - RELEASING step 8: the asset list grows to six archives;
  - the CHANGELOG.
- **Then release `0.1.0-beta.7`** through RELEASING. The main session tags it after the user's
  go, and verifies the musl assets with the installer on `amazonlinux:2`.

### Task 2: Homebrew tap

- **The user, in the browser and the terminal** (the main session prints the exact steps as an
  interactive checklist):
  1. Create the public repo `redis-pane/homebrew-tap`, empty, or with a README only.
  2. Create a fine-grained token scoped to that repo alone, with Contents: read and write.
  3. Store it as the secret `HOMEBREW_TAP_TOKEN` on `redis-pane/redis-pane`
     (`gh secret set HOMEBREW_TAP_TOKEN -R redis-pane/redis-pane`).
- **The subagent:**
  - `installers` gains `"homebrew"`. Set `tap = "redis-pane/homebrew-tap"` and
    `publish-jobs = ["homebrew"]`, plus the formula name and description.
  - `dist` does not publish prereleases to the tap by default. Turn that on for the betas
    (`publish-prereleases = true`, or the 0.32 equivalent; check the docs), so the tap can be
    verified before 1.0. Record whether it stays on afterwards.
  - Run `dist generate`.
  - **Docs:**
    - the installation page and the README lead with `brew install redis-pane/tap/redis-pane` on
      macOS (and on Linux with Homebrew);
    - RELEASING step 8 gains "the tap's formula updated to the new version, and `brew install`
      on a clean machine";
    - the CHANGELOG.
- **Then release `0.1.0-beta.8`.**
  - Check that the tap gets the formula commit.
  - Check that `brew install redis-pane/tap/redis-pane` works on macOS and in a Linux Homebrew
    container, and that `brew test` passes if a test block is generated.

### Task 3: the 1.0 gate and close-out

- **Check the gate, and write the evidence into this plan:**
  - Every channel, installed on a clean machine, with the version shown.
  - `docs/UI_TASKS.md` has no open Severity 1 or 2.
  - `gh issue list --label bug --state open` is empty. If anything is open, stop and report.
    Fixing it is a separate decision.
- **Version `1.0.0`.** Run RELEASING in full. Make the "beta" wording accurate everywhere:
  - README, `TESTING.md` (retire it or retitle it, and say which), and `CLAUDE.md`;
  - the site's landing page and `site/lib/release.ts`;
  - the bug form placeholder;
  - the installation page's unsigned-binary note: binaries are still unsigned, so keep it,
    reworded without "beta".
- **PRD §9 and PLAN:** mark 1.0 done. The README's "Expect rough edges" goes.
- **The tag waits for the user.** After it, verify every channel again.

## Shared rules

- Never hand-edit generated files: `release.yml` from `dist generate`, and the docs references.
- Never remove Docker containers you didn't create. Name the verification containers
  `redis-pane-pkg-*` and remove only those.
- No repo-settings changes and no secrets. Those are the user's (task 2).
- Commit messages end with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. PR bodies
  end with `🤖 Generated with [Claude Code](https://claude.com/claude-code)`. Pass the body with
  `--body-file`, and edit it with `gh api`.

## Risks

- **musl perf** is mitigated by the allocator step, measured not assumed.
- **Cross-building `ring` for aarch64 musl:** if `dist` can't, fall back to a native arm runner.
  If neither works, ship x86_64 musl only and record arm64 as a follow-up. That doesn't block
  1.0 unless the user says so.
- **The tap token expires.** A fine-grained token has an expiry date. RELEASING must say how to
  renew it, and the publish job fails loudly when it has expired.
