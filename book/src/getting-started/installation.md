# Installation

## Release binaries

Download the binary for your system from the
[Releases page](https://github.com/vinodsantharam/redis-pane/releases). There are builds for macOS
(Intel and Apple Silicon), Linux (x86_64) and Windows.

Or run the installer script. The beta releases are marked as GitHub prereleases, so the
`/latest/` URL doesn't resolve to them. Use the tagged URL and change the tag to the current
one from the Releases page.

On macOS and Linux:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/vinodsantharam/redis-pane/releases/download/v0.1.0-beta.4/redis-pane-installer.sh | sh
```

On Windows, from PowerShell:

```powershell
irm https://github.com/vinodsantharam/redis-pane/releases/download/v0.1.0-beta.4/redis-pane-installer.ps1 | iex
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
git clone https://github.com/vinodsantharam/redis-pane.git
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
