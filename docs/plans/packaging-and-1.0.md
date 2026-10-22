# Packaging, and the road to 1.0

Status: **planned** 2026-10-21; task 1 **done** (PR #129); task 2 subagent half **done**, pending verification after `beta.8`. Four tasks, in order. Tasks 1, 3 and 4 run on one Sonnet
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

#### Task 1 results (2026-10-21)

**dist configuration.** `targets` gains both musl triples; the gnu x86_64 target stays. dist 0.32
picks a native `ubuntu-22.04-arm` runner for aarch64 by itself and installs `musl-tools` for both
musl builds, so `release.yml` needed no hand edit and `ring` builds without a cross toolchain. The
only other change is `[dist.min-glibc-version] x86_64-unknown-linux-gnu = "2.34"` (below).

**How the shell installer chooses (read from the generated installer).** `get_architecture` reports
`gnu` unless `ldd --version` mentions musl (`musl-dynamic`). For x86_64 gnu it runs
`check_glibc <major> <series>` and uses the gnu archive only if the host's glibc is at least that,
else falls back to the x86_64 musl archive. A musl host gets musl. Every aarch64 Linux, gnu or
musl, gets the aarch64 musl archive, since no gnu arm64 build exists. **dist's default floor is
2.31, but the build links glibc 2.34 symbols** (built on ubuntu-22.04): Debian 11 and Ubuntu 20.04
(glibc 2.31) would have been sent the gnu build, which fails there with `GLIBC_2.32/33/34 not
found`. Fixed with the `min-glibc-version` entry above; the regenerated installer calls
`check_glibc "2" "34"`, and run on `debian:11` it falls back to musl, on `ubuntu:22.04` it keeps gnu.

**Perf, musl vs gnu.** Native `linux/arm64` containers on one host, back to back: gnu in
`rust:1-bookworm`, musl in `rust:1-alpine`, same command
(`cargo test -p redis-pane-core --release --test perf -- --ignored --nocapture --test-threads=1`),
median of 3 runs (mimalloc: 2 runs). Times in ms; the ratio is to gnu. This is aarch64 only:
x86_64 under emulation was not used.

| Figure @ 1M keys | gnu | musl | musl / gnu | musl + mimalloc | / gnu |
|---|---|---|---|---|---|
| Space toggle (update) | 0.011 | 0.025 | 2.27 | 0.009 | 0.79 |
| scroll keystroke | 0.087 | 0.140 | 1.61 | 0.076 | 0.87 |
| ScanBatch of 500 | 0.104 | 0.164 | 1.57 | 0.086 | 0.82 |
| ScanBatch of 500, tree mode | 0.099 | 0.131 | 1.33 | 0.081 | 0.82 |
| filter keystroke | 0.322 | 0.359 | 1.12 | 0.320 | 0.99 |
| filter first character | 4.06 | 4.01 | 0.99 | 4.19 | 1.03 |
| name sort alone (sorted) | 26.6 | 23.8 | 0.90 | 24.1 | 0.90 |
| name sort alone (random deep) | 98.6 | 83.8 | 0.85 | 78.6 | 0.80 |
| fold alone | 23.5 | 18.4 | 0.78 | 18.3 | 0.78 |
| sort change to new list (random deep) | 118.0 | 105.1 | 0.89 | 109.2 | 0.93 |
| tree toggle to new list (random deep) | 157.1 | 137.4 | 0.87 | 137.9 | 0.88 |
| whole-scan fold, tree mode | 72.7 | 64.1 | 0.88 | 61.6 | 0.85 |

- **Verdict: the rule fired.** Three figures were over 1.5x gnu (Space, scroll, `ScanBatch`; plain
  musl is slower on the many-small-allocations paths and faster or equal on the big-sort ones). All
  are tens to hundreds of microseconds against a 16ms frame, but the rule is the rule, so the app
  crate now uses `mimalloc` as the global allocator on `target_env = "musl"` only
  (`crates/app/src/main.rs`; the core never names an allocator). Re-measured, every figure is at
  or below gnu except noise on `render_alone`, which is 0.04ms either way.
- **The perf suite does not link the app's allocator,** so the re-measure put the same
  `#[global_allocator]` in `perf.rs` temporarily (reverted, not committed). Remeasure with the same
  trick if the allocator ever changes.
- **`name_sort_alone_at_1m_keys` fails its 21ms ceiling in this Docker VM on gnu and musl alike**
  (26.6 and 23.8ms; it passes on CI hardware). It is not a musl signal, and every other budget
  passes.

**Binary size** (dist profile, from the CI artifacts, stripped): x86_64 musl 12.1MB (3.2MB
archive), aarch64 musl 10.8MB (2.8MB archive), x86_64 gnu 11.8MB. All under the 20MB budget.

**Old distros** (musl binaries downloaded from the PR's Release run, `--version` and `--probe`
against `redis:8.4-alpine` on a shared network; every cell passed, `--probe` printing
`redis 8.4.6` and `liveness: CLIENT TRACKING accepted`):

| Image | x86_64 (emulated) | arm64 (native) |
|---|---|---|
| `amazonlinux:2` (glibc 2.26) | pass | pass |
| `rockylinux:8` (glibc 2.28) | pass | pass |
| `debian:11` (glibc 2.31) | pass | pass |
| `alpine:3` (musl) | pass | pass |

The gnu x86_64 build fails on `amazonlinux:2` (needs GLIBC_2.29 to 2.34), on `rockylinux:8`
(2.29 to 2.34) and on `debian:11` (2.32 to 2.34), as the plan predicted.

**TLS.** A TLS-enabled `redis:8.4-alpine` with a private CA. Without the CA in the system bundle,
`--probe` ends with `InvalidCertificate(UnknownIssuer)`; after appending the CA to the distro's own
bundle (`/etc/pki/tls/certs/ca-bundle.crt` or `/etc/ssl/certs/ca-certificates.crt`) it connects.
Passed on `amazonlinux:2` x86_64, `rockylinux:8` arm64, `debian:11` x86_64 and `alpine:3` arm64,
so the static build does find the system CA bundle. A minimal `debian:11` has no bundle at all
until `ca-certificates` is installed; the installation page says so.

**Not done here.** The shell installer was not run end to end in the containers: it downloads from
a release URL, and none exists until the tag. The main session checks it on `amazonlinux:2` after
tagging (RELEASING step 8).

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

#### Task 2 results (subagent half, 2026-10-22)

Status: **pending verification after `beta.8`.** Nothing has been published; the formula has only
been generated locally.

- **Config (`dist-workspace.toml`):** `installers` gains `"homebrew"`; `tap = "redis-pane/homebrew-tap"`,
  `formula = "redis-pane"`, `publish-jobs = ["homebrew"]`, `publish-prereleases = true`. The
  description and homepage come from `crates/app/Cargo.toml`, which had neither; `license` comes
  from the workspace (`MIT`). `dist generate` added the `publish-homebrew-formula` job to `release.yml`
  with `token: ${{ secrets.HOMEBREW_TAP_TOKEN }}`, dist's default secret name.
- **Prereleases.** Without the setting the job is gated by
  `!announcement_is_prerelease || publish_prereleases`, so beta tags would skip the tap. The generated
  workflow keeps that condition and the plan JSON carries `publish_prereleases`, which comes from
  `publish-prereleases = true`. It is only accepted in the workspace `[dist]` table, which is
  where it is. **Keep it on until 1.0 is verified;** at `1.0.0` the tag is stable, so the setting no
  longer matters and can be removed or left.
- **Formula, generated locally** (`dist build --artifacts=global`, then `target/distrib/redis-pane.rb`):
  macOS arm64 and x86_64 take the `apple-darwin` archives. dist offers no setting to choose between
  gnu and musl: with a gnu x86_64 target built, the formula took gnu, which fails on Linuxbrew hosts
  with glibc below 2.34. **Decision (user, 2026-10-22): Linux ships musl only.**
  `x86_64-unknown-linux-gnu` and `[dist.min-glibc-version]` were removed (ADR-0023 amendment). The
  regenerated formula uses `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. In the
  regenerated shell installer every Linux variant (gnu, musl dynamic or static) maps to the musl
  archive for its architecture, and `check_glibc` is defined but never called.
- **x86_64 musl perf was never measured natively.** Task 1 measured on arm64, where musl with
  mimalloc was at or below gnu. It is not measured under emulation.
- **No `test do` block** is generated by dist 0.32, so `brew test` has nothing to run. Locally the
  formula also has no `sha256` lines: those are filled in the release run, once the archives'
  checksums exist. Verify them on the tap after `beta.8`.
- **`brew style`** on the local file: 14 offenses, all from dist's template (hash alignment,
  Sorbet sigils, a mutable constant, a missing frozen-string comment, documentation). The
  workflow runs `brew style --fix` with `Desc` and `Homepage` excepted before committing. The
  description is kept under 80 characters so `FormulaAudit/Desc` passes anyway.
- **Still to do after `beta.8`:** the tap has the formula commit; `brew install
  redis-pane/tap/redis-pane` works on macOS and in a Linux Homebrew container; the `sha256`
  lines are present; whether Homebrew avoids the Gatekeeper prompt (untested).

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
