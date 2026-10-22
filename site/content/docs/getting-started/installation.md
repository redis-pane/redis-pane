---
title: "Installation"
---

## Release binaries

Download the binary for your system from the
[Releases page](https://github.com/redis-pane/redis-pane/releases). There are builds for macOS
(Intel and Apple Silicon), Linux (x86_64 and arm64) and Windows.

Linux has three builds:

| Archive | Runs on |
|---|---|
| `redis-pane-x86_64-unknown-linux-musl` | any x86_64 Linux, including old ones. It is a static binary with no dependency on the system's C library. |
| `redis-pane-aarch64-unknown-linux-musl` | any arm64 Linux, the same way. |
| `redis-pane-x86_64-unknown-linux-gnu` | x86_64 Linux with glibc 2.34 or newer (Ubuntu 22.04, Debian 12, RHEL 9 and later). |

The static builds run on the hosts that are common for bastions and old servers: Amazon Linux 2,
RHEL and Rocky 8, Debian 11 and Alpine. The gnu build does not run there, and fails with
`GLIBC_2.34 not found`. If you download by hand and are unsure, take the musl one.
The installer script chooses for you: it uses the gnu build only where glibc is new enough, and
the musl build everywhere else, including every arm64 Linux.

A static build reads the system's CA bundle for `rediss://` connections, as the gnu one does. On a
minimal image, install your distro's `ca-certificates` package first.

Or run the installer script. The beta releases are marked as GitHub prereleases, so the
`/latest/` URL doesn't resolve to them. Use the tagged URL and change the tag to the current
one from the Releases page.

On macOS and Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/redis-pane/redis-pane/releases/download/v0.1.0-beta.7/redis-pane-installer.sh | sh
```

On Windows, from PowerShell:

```powershell
irm https://github.com/redis-pane/redis-pane/releases/download/v0.1.0-beta.7/redis-pane-installer.ps1 | iex
```

## The first run of an unsigned binary

The beta builds are not signed, so your system may refuse them the first time.

- **macOS** says the app is from an unidentified developer. Right-click the binary, choose
  Open, and confirm. Or run `xattr -d com.apple.quarantine ./redis-pane` once.
- **Windows** SmartScreen shows "Windows protected your PC". Click "More info", then
  "Run anyway".

## From source

You need [Rust](https://rustup.rs).

```bash
git clone https://github.com/redis-pane/redis-pane.git
cd redis-pane
cargo build --release
```

The binary is at `./target/release/redis-pane`.

## Check it works

```bash
redis-pane --version
redis-pane --print-target
```

`--print-target` resolves where `redis-pane` would connect and exits without connecting. With
nothing configured it prints `127.0.0.1:6379/0 · local · from default`.
