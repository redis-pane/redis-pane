//! The command line, defined once so the binary and the generated CLI
//! reference (`book/src/reference/cli.md`) read the same clap definition.

use clap::{CommandFactory, Parser};

/// A terminal UI for Redis.
///
/// With no arguments, resolves a target from the default Profile, then the
/// environment, then `127.0.0.1:6379`. It never prompts; the title bar always
/// shows what was chosen and why (ADR-0001).
#[derive(Debug, Parser)]
#[command(name = "redis-pane", version, about, long_about = None)]
pub struct Cli {
    /// Profile to use, by name. A bare positional name works too.
    #[arg(long, value_name = "NAME")]
    pub profile: Option<String>,
    /// Profile name, positionally.
    #[arg(value_name = "PROFILE")]
    pub positional_profile: Option<String>,
    /// Connect to this URL, used wholesale.
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    #[arg(long, value_name = "HOST")]
    pub host: Option<String>,
    #[arg(long, value_name = "PORT")]
    pub port: Option<u16>,
    /// Database index. Fixed at launch; there is no in-app switcher (ADR-0005).
    #[arg(long, value_name = "N")]
    pub db: Option<u8>,
    /// ACL username. Always wins over a Profile's or the environment's.
    #[arg(long, value_name = "NAME")]
    pub user: Option<String>,
    /// Password, given directly. Visible in shell history and to other users
    /// via `ps` — prefer a Profile's `passwordEnv`/`passwordCommand` for
    /// anything long-lived. Always wins over a Profile's or the environment's.
    #[arg(long, value_name = "SECRET")]
    pub password: Option<String>,
    /// Use TLS. Additive only — there is no --no-tls to downgrade a Profile or
    /// a `rediss://` URL that already wants it.
    #[arg(long)]
    pub tls: bool,
    /// Draw with ASCII glyphs only. The default follows the locale: ASCII
    /// unless it declares UTF-8. Overrides the config file's `ascii`.
    #[arg(long, conflicts_with = "unicode")]
    pub ascii: bool,
    /// Draw with Unicode glyphs even where the locale does not declare UTF-8.
    #[arg(long)]
    pub unicode: bool,
    /// Resolve and print the target, then exit without connecting.
    #[arg(long)]
    pub print_target: bool,
    /// Connect, report what the server supports, then exit.
    #[arg(long)]
    pub probe: bool,
    /// Colour theme: `dark`, `light`, `high-contrast`, or one defined under
    /// `themes` in the config. Beats the config's `theme`.
    #[arg(long, value_name = "NAME")]
    pub theme: Option<String>,
}

/// The clap command, for rendering the CLI reference.
pub fn cli_command() -> clap::Command {
    Cli::command()
}
