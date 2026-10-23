---
title: "Installation"
---

## Homebrew

On macOS, and on Linux with [Homebrew](https://brew.sh):

```bash
brew install redis-pane/tap/redis-pane
```

The formula installs the same release binaries described below: the static musl build on Linux.
On macOS a Homebrew install doesn't get the first-run prompt for an unsigned binary: Homebrew
doesn't mark what it downloads as quarantined, so the note under "The first run of an unsigned
binary" applies only to the release binaries and the install script.

## Release binaries

Download the binary for your system from the
[Releases page](https://github.com/redis-pane/redis-pane/releases). There are builds for macOS
(Intel and Apple Silicon), Linux (x86_64 and arm64) and Windows (experimental, see
[Limits](../limits.md)).

Linux builds are static musl binaries, one for x86_64 and one for arm64:

| Archive | Runs on |
|---|---|
| `redis-pane-x86_64-unknown-linux-musl` | any x86_64 Linux distro, including old ones. It has no dependency on the system's C library. |
| `redis-pane-aarch64-unknown-linux-musl` | any arm64 Linux distro, the same way. |

They run on the hosts that are common for bastions and old servers: Amazon Linux 2, RHEL and
Rocky 8, Debian 11 and Alpine. The installer script picks the right one for you.

A static build reads the system's CA bundle for `rediss://` connections, from the system. On a
minimal image, install your distro's `ca-certificates` package first.

Or run the installer script. The URLs below always fetch the latest release. To install a specific
version, replace `latest/download` with `download/v` and a version from the Releases page, for
example `download/v1.0.0`.

On macOS and Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/redis-pane/redis-pane/releases/latest/download/redis-pane-installer.sh | sh
```

On Windows (experimental), from PowerShell:

```powershell
irm https://github.com/redis-pane/redis-pane/releases/latest/download/redis-pane-installer.ps1 | iex
```

## The first run of an unsigned binary

The binaries are not signed, so your system may refuse them the first time.

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
