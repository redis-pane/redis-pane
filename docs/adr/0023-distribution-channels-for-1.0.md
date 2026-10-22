# ADR-0023 — Distribution channels for 1.0

**Status:** Accepted · **Date:** 2026-10-21

## Context

Through `0.1.0-beta.6`, `redis-pane` shipped only what `cargo-dist` produced by default: four
unsigned archives (macOS arm64 and x86_64, Linux x86_64 gnu, Windows x86_64) and shell and
PowerShell installers. PRD §10 left the rest to a decision "at the end of alpha". That decision
was still open six betas later, and 1.0 needs it.

The product is meant to be used where engineers actually work, which includes SSH sessions on
bastion hosts (PRD §1). Two facts follow from that:

- **The gnu Linux build doesn't run on many bastions.** It is built on `ubuntu-22.04` against
  glibc 2.35. RHEL 8 (glibc 2.28) and Amazon Linux 2 (2.26) refuse it with
  `GLIBC_2.3x not found`, and those are common bastion images.
- **A static musl build is cheap.** TLS is `rustls` with `ring`, and nothing links OpenSSL. musl's
  allocator is slow, though, so the 1M-key perf budgets have to be re-measured on it.

## Decision

**1.0 ships through four channels:**

1. **The GitHub Release archives,** extended with `x86_64-unknown-linux-musl` and
   `aarch64-unknown-linux-musl`.
   - The gnu x86_64 build stays, for hosts whose glibc is new enough.
   - The musl builds use `mimalloc` only if the perf suite shows they need it.
2. **The shell and PowerShell installers,** as today.
3. **A Homebrew tap,** `redis-pane/homebrew-tap`, published by `cargo-dist`'s Homebrew job:
   `brew install redis-pane/tap/redis-pane`, on macOS and on Linux with Homebrew.

**Binaries stay unsigned for 1.0.** The documented first-run workaround stays.

## Rejected for 1.0

- **macOS signing and notarization.** It needs an Apple Developer account ($99 a year) and four
  CI secrets. A Homebrew install doesn't trigger the Gatekeeper prompt for most users, so the tap
  removes most of the friction signing would. Revisit after 1.0.
- **Windows signing.** A code-signing certificate costs hundreds a year, and the audience is
  overwhelmingly macOS and Linux.
- **crates.io.** A second release surface, with two crates (`redis-pane-core` and `redis-pane`)
  to publish in order, and a name to hold. Cargo users can already run the shell installer.
- **gnu arm64 Linux.** musl arm64 runs everywhere it would.
- **Distro packages (apt, rpm, AUR) and Scoop/winget.** Each is a separate pipeline to maintain.
  Static binaries plus an installer cover the same hosts.

## Consequences

- The release has six platform archives instead of four. RELEASING's verification step lists them.
- Releasing depends on a repo secret, `HOMEBREW_TAP_TOKEN`: a fine-grained token with an expiry
  date. RELEASING says how to renew it, and an expired token fails the publish job loudly rather
  than skipping the tap.
- The installation docs lead with Homebrew on macOS and say which Linux build runs where.
- Plan: [`plans/packaging-and-1.0.md`](../plans/packaging-and-1.0.md).

## Amendment, 2026-10-22: Linux is musl only

Linux ships static musl builds only, for x86_64 and arm64. The `x86_64-unknown-linux-gnu` build is
dropped, and the release has five platform archives.

A Homebrew formula cannot choose musl while a gnu build exists: dist prefers gnu for x86_64 Linux,
so Linuxbrew hosts with an old glibc would get a binary that cannot start. musl with `mimalloc`
matched gnu on task 1's measurements (on arm64; x86_64 musl was not measured natively). The shell
installer now sends every Linux host to musl, with no glibc check.
