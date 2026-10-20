//! Connection resolution — pure (ADR-0001, R1.2, PLAN M0.6).
//!
//! Strict precedence: explicit flags beat the config file's default Profile,
//! which beats the environment, which falls back to `127.0.0.1:6379`.
//!
//! Two rules carry the weight here:
//!
//! 1. **It never prompts.** Where sources conflict the app resolves
//!    deterministically, and the title bar permanently shows the resolved
//!    target *and* its [`Source`]. That readout is the entire mitigation for
//!    resolving silently — it is not optional chrome.
//! 2. **`REDIS_URL` is used wholesale or not at all.** It is never merged at
//!    the component level with `REDIS_HOST`/`REDIS_PORT`/`REDIS_USER`/
//!    `REDIS_PASSWORD`, because a stale export lingering in a shell would then
//!    produce a target nobody can explain.

use crate::config::{Config, Profile};
use crate::state::{Connection, Environment, Source};

/// Where a password comes from — a *description*, not the secret itself.
///
/// Secrets are references (ADR-0002), and resolving a reference means reading
/// an environment variable or running a command, both of which are I/O. So the
/// core says what to fetch and the shell fetches it. A literal passes through
/// only because the config file is refused when it is group- or world-readable.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PasswordSource {
    #[default]
    None,
    Literal(String),
    /// The name of an environment variable holding the password.
    Env(String),
    /// A command whose stdout is the password.
    Command(String),
}

/// What is needed to authenticate, as references.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Credentials {
    pub username: Option<String>,
    pub password: PasswordSource,
    pub tls: bool,
}

/// Everything resolution produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    pub connection: Connection,
    pub credentials: Credentials,
    /// The URL to actually dial, credentials intact.
    ///
    /// Kept apart from [`Connection::target`], which is redacted for display.
    /// Reconstructing one from the other would mean either dialling a redacted
    /// URL or showing a password.
    pub dial_url: String,
}

impl Resolution {
    pub fn dial_url(&self) -> &str {
        &self.dial_url
    }
}

/// What the user asked for on the command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flags {
    pub url: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub db: Option<u8>,
    pub profile: Option<String>,
    /// `--user`, `--password`, `--tls` — a credential source that always wins,
    /// on top of whatever target and credentials the other flags, a Profile,
    /// or the environment produced. See `resolve()`'s override pass below.
    pub user: Option<String>,
    pub password: Option<String>,
    pub tls: bool,
}

impl Flags {
    fn names_a_target(&self) -> bool {
        self.url.is_some() || self.host.is_some() || self.port.is_some()
    }
}

/// The environment variables resolution looks at. Passed in as data so that
/// resolution stays pure and the precedence matrix is testable as a table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvVars {
    pub redis_url: Option<String>,
    pub redis_host: Option<String>,
    pub redis_port: Option<String>,
    pub redis_user: Option<String>,
    pub redis_password: Option<String>,
}

impl EnvVars {
    fn names_a_target(&self) -> bool {
        self.redis_url.is_some() || self.redis_host.is_some() || self.redis_port.is_some()
    }
}

const DEFAULT_HOST: &str = "127.0.0.1";
const DEFAULT_PORT: u16 = 6379;

/// Why resolution refused to pick a target.
///
/// A Profile named on the command line that the config does not define, or a
/// Cluster URL asked for a database other than 0. Falling through to the next rule would connect somewhere the
/// user did not ask for, and a typo on the command line deserves the same
/// refusal as a typo in the hand-authored config (ADR-0001, ADR-0002).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    UnknownProfile {
        name: String,
        /// Profile names defined by the config, sorted. Empty when there is no
        /// config file or it defines none.
        available: Vec<String>,
        /// Whether a config file was loaded at all.
        has_config: bool,
    },
    /// A Cluster URL whose effective database is not 0. A Cluster has one
    /// logical database, so the request cannot be honoured; refusing is the only
    /// way the title bar's `/db` stays true (ADR-0022).
    ClusterDatabase { target: String, db: u8 },
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::ClusterDatabase { target, db } => write!(
                f,
                "{target} is a Redis Cluster, which has only database 0; database {db} was requested"
            ),
            ResolveError::UnknownProfile {
                name,
                available,
                has_config,
            } => {
                if !*has_config {
                    write!(
                        f,
                        "no Profile named \"{name}\": there is no config file, so no Profiles are defined"
                    )
                } else if available.is_empty() {
                    write!(
                        f,
                        "no Profile named \"{name}\": the config file defines no Profiles"
                    )
                } else {
                    write!(
                        f,
                        "no Profile named \"{name}\"; available Profiles: {}",
                        available.join(", ")
                    )
                }
            }
        }
    }
}

impl std::error::Error for ResolveError {}

/// Resolve a Connection from flags, config and environment.
///
/// Never prompts. It fails only when `--profile` (or the positional name)
/// names a Profile the config does not define; otherwise the fallback is
/// always `127.0.0.1:6379`.
pub fn resolve(
    flags: &Flags,
    config: Option<&Config>,
    env: &EnvVars,
) -> Result<Resolution, ResolveError> {
    if let Some(name) = &flags.profile
        && !config.is_some_and(|c| c.profiles.contains_key(name))
    {
        let mut available: Vec<String> = config
            .map(|c| c.profiles.keys().cloned().collect())
            .unwrap_or_default();
        available.sort();
        return Err(ResolveError::UnknownProfile {
            name: name.clone(),
            available,
            has_config: config.is_some(),
        });
    }
    let mut resolution = resolve_target(flags, config, env)?;
    // `--user`/`--password`/`--tls` always win, on top of whatever the target
    // resolution above produced — a Profile's stored password included. This
    // is the same "explicit flags beat everything" precedence ADR-0001 already
    // states for the target itself, extended to credentials.
    if let Some(user) = &flags.user {
        resolution.credentials.username = Some(user.clone());
    }
    if let Some(password) = &flags.password {
        resolution.credentials.password = PasswordSource::Literal(password.clone());
    }
    if flags.tls {
        resolution.credentials.tls = true;
    }
    Ok(resolution)
}

fn resolve_target(
    flags: &Flags,
    config: Option<&Config>,
    env: &EnvVars,
) -> Result<Resolution, ResolveError> {
    // 1. An explicit --profile names a Profile, and outranks everything but a
    //    target given directly on the command line.
    if let Some(name) = &flags.profile
        && let Some(profile) = config.and_then(|c| c.profiles.get(name))
    {
        return from_profile(name, profile, flags.db);
    }

    // 2. A target given directly on the command line.
    if flags.names_a_target() {
        let (target, dial_url) = match &flags.url {
            Some(url) => url_target(url, flags.db)?,
            None => {
                let target = format_target(
                    flags.host.as_deref().unwrap_or(DEFAULT_HOST),
                    flags.port.unwrap_or(DEFAULT_PORT),
                    flags.db.unwrap_or(0),
                );
                let dial_url = dial_from(None, &target);
                (target, dial_url)
            }
        };
        let environment = infer_environment(&target);
        return Ok(Resolution {
            connection: Connection {
                target,
                environment,
                source: Source::Flag,
                topology: None,
            },
            credentials: env_credentials(env),
            dial_url,
        });
    }

    // 3. The config file's default Profile.
    if let Some(config) = config
        && let Some(name) = &config.default_profile
        && let Some(profile) = config.profiles.get(name)
    {
        return from_profile(name, profile, flags.db);
    }

    // 4. The environment. REDIS_URL wholesale, or the discrete set — never both.
    if env.names_a_target() {
        let (target, dial_url) = match &env.redis_url {
            Some(url) => url_target(url, flags.db)?,
            None => {
                let target = format_target(
                    env.redis_host.as_deref().unwrap_or(DEFAULT_HOST),
                    env.redis_port
                        .as_deref()
                        .and_then(|p| p.parse().ok())
                        .unwrap_or(DEFAULT_PORT),
                    flags.db.unwrap_or(0),
                );
                let dial_url = dial_from(None, &target);
                (target, dial_url)
            }
        };
        let environment = infer_environment(&target);
        return Ok(Resolution {
            connection: Connection {
                target,
                environment,
                source: Source::Environment,
                topology: None,
            },
            credentials: env_credentials(env),
            dial_url,
        });
    }

    // 5. Nothing was configured.
    let target = format_target(DEFAULT_HOST, DEFAULT_PORT, flags.db.unwrap_or(0));
    let dial_url = dial_from(None, &target);
    Ok(Resolution {
        connection: Connection {
            target,
            environment: Environment::Local,
            source: Source::Default,
            topology: None,
        },
        credentials: env_credentials(env),
        dial_url,
    })
}

/// The URL to dial, assembled from a host target. The displayed target is
/// redacted, so for a URL target [`url_target`] builds the dial URL instead.
fn dial_from(raw: Option<&str>, target: &str) -> String {
    match raw {
        Some(url) => url.to_string(),
        None if target.contains("://") => target.to_string(),
        None => format!("redis://{target}"),
    }
}

/// The displayed target and the dial URL for a URL target.
///
/// One effective database governs both: `chosen` (`--db`, or a Profile's `db`)
/// if given, else the URL's own, else 0. The displayed target is the redacted
/// URL carrying exactly that `/db`, so the title bar says the database the
/// connection uses (#106). The dial URL is the original, rewritten only when
/// the effective database differs from what the URL already says, so a URL
/// nobody overrode is dialled byte for byte as written.
///
/// A Cluster has only database 0, so a `redis-cluster://` or
/// `rediss-cluster://` URL whose effective database is not 0 is refused.
fn url_target(url: &str, chosen: Option<u8>) -> Result<(String, String), ResolveError> {
    let own = url_database(url);
    let db = chosen.or(own).unwrap_or(0);
    if db != 0 && is_cluster_url(url) {
        return Err(ResolveError::ClusterDatabase {
            target: redact(url),
            db,
        });
    }
    let dial = if own.unwrap_or(0) == db {
        url.to_string()
    } else {
        with_database(url, db)
    };
    Ok((redact(&with_database(url, db)), dial))
}

fn is_cluster_url(url: &str) -> bool {
    url.starts_with("redis-cluster://") || url.starts_with("rediss-cluster://")
}

/// Where the path of a `scheme://authority/path?query#fragment` URL sits, as a
/// range into `url`; `None` for a string without `://` and for a unix-socket
/// URL, whose path is the socket and not a database.
fn path_span(url: &str) -> Option<std::ops::Range<usize>> {
    let (scheme, rest) = url.split_once("://")?;
    if scheme.contains("unix") {
        return None;
    }
    let rest_start = scheme.len() + 3;
    // The authority (userinfo, host list) ends at the first `/`, `?` or `#`;
    // a password holding one of them must be percent-encoded, as in any URL.
    let delim = |s: &str| s.find(['/', '?', '#']);
    let start = rest_start + delim(rest).unwrap_or(rest.len());
    let tail = &url[start..];
    let end = start + tail.find(['?', '#']).unwrap_or(tail.len());
    Some(start..end)
}

/// The database a URL names in its path (`/2`), if it names one.
fn url_database(url: &str) -> Option<u8> {
    let span = path_span(url)?;
    url[span].trim_start_matches('/').parse().ok()
}

/// `url` with its path replaced by `/<db>`, keeping the scheme, userinfo, host
/// list, query and fragment exactly as written. A URL with no path-database
/// notion (unix socket, no scheme) is returned as it came.
fn with_database(url: &str, db: u8) -> String {
    match path_span(url) {
        Some(span) => format!("{}/{db}{}", &url[..span.start], &url[span.end..]),
        None => url.to_string(),
    }
}

/// `REDIS_USER` and `REDIS_PASSWORD`. Used only when the target itself came
/// from the environment or the command line — never merged into a Profile,
/// whose credentials are stated in the file (ADR-0001).
fn env_credentials(env: &EnvVars) -> Credentials {
    Credentials {
        username: env.redis_user.clone(),
        password: match &env.redis_password {
            Some(p) => PasswordSource::Literal(p.clone()),
            None => PasswordSource::None,
        },
        tls: false,
    }
}

fn from_profile(
    name: &str,
    profile: &Profile,
    db_override: Option<u8>,
) -> Result<Resolution, ResolveError> {
    // `--db`, then the Profile's `db`, then (for a URL) the URL's own, then 0.
    let chosen = db_override.or(profile.db);
    let (target, dial_url) = match &profile.url {
        Some(url) => url_target(url, chosen)?,
        None => {
            let target = format_target(
                profile.host.as_deref().unwrap_or(DEFAULT_HOST),
                profile.port.unwrap_or(DEFAULT_PORT),
                chosen.unwrap_or(0),
            );
            let dial_url = dial_from(None, &target);
            (target, dial_url)
        }
    };
    // Reference forms are checked before the literal, so a Profile carrying
    // both does not silently use the one ADR-0002 discourages.
    let password = if let Some(var) = &profile.password_env {
        PasswordSource::Env(var.clone())
    } else if let Some(cmd) = &profile.password_command {
        PasswordSource::Command(cmd.clone())
    } else if let Some(literal) = &profile.password {
        PasswordSource::Literal(literal.clone())
    } else {
        PasswordSource::None
    };

    Ok(Resolution {
        connection: Connection {
            target,
            environment: profile.environment(),
            source: Source::Profile(name.to_string()),
            topology: None,
        },
        credentials: Credentials {
            username: profile.username.clone(),
            password,
            tls: profile.tls.unwrap_or(false),
        },
        dial_url,
    })
}

fn format_target(host: &str, port: u16, db: u8) -> String {
    format!("{host}:{port}/{db}")
}

/// Remove any password embedded in a URL.
///
/// The title bar shows the target permanently (ADR-0001), so a password in a
/// `rediss://user:pass@host` URL would sit on screen for the whole session —
/// through every screen-share, screenshot and pasted diagnostic. The userinfo
/// is kept because a username is not a secret and identifies which ACL user is
/// connected, which is exactly the kind of thing the Source readout exists for.
pub fn redact(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let Some((userinfo, host)) = rest.split_once('@') else {
        return url.to_string();
    };
    match userinfo.split_once(':') {
        Some((user, _password)) if !user.is_empty() => format!("{scheme}://{user}:•••@{host}"),
        // `redis://:password@host` — no username, so nothing worth keeping.
        Some(_) => format!("{scheme}://•••@{host}"),
        None => url.to_string(),
    }
}

/// An ad-hoc target that is not loopback or a unix socket is `unknown`, and
/// therefore starts in Read-only Mode (ADR-0004). `unknown` is a real
/// Environment, not a stand-in for "probably fine".
///
/// Strips every scheme `fred::Config::from_url` accepts, not just the two
/// plain ones — a loopback `redis-sentinel://` or `redis-cluster://` URL must
/// resolve to `Local` exactly like a plain `redis://127.0.0.1` does (ADR-0022;
/// first handled in `docs/plans/m4-cluster-refusal.md`). The `-cluster`/`-sentinel` variants
/// are stripped first so e.g. `redis-cluster://` is never left with a
/// dangling `-cluster` prefix after only the plain scheme is stripped.
fn infer_environment(target: &str) -> Environment {
    let host = target.rsplit('@').next().unwrap_or(target);
    let host = [
        "rediss-cluster://",
        "redis-cluster://",
        "rediss-sentinel://",
        "redis-sentinel://",
        "rediss://",
        "redis://",
    ]
    .iter()
    .fold(host, |h, scheme| h.trim_start_matches(scheme));
    let is_loopback = host.starts_with("127.0.0.1")
        || host.starts_with("localhost")
        || host.starts_with("[::1]")
        || host.starts_with("::1");
    let is_unix_socket = target.starts_with('/') || target.starts_with("unix:");
    if is_loopback || is_unix_socket {
        Environment::Local
    } else {
        Environment::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `resolve`, for cases where the Profile is known to exist.
    fn resolved(flags: &Flags, config: Option<&Config>, env: &EnvVars) -> Resolution {
        resolve(flags, config, env).expect("resolves")
    }

    fn config_with_default() -> Config {
        crate::config::parse(
            r#"{
                 "defaultProfile": "staging",
                 "profiles": {
                   "staging": {"host":"cache-01","port":6379,"env":"staging"},
                   "prod":    {"host":"redis.prod","port":6380,"db":2,"env":"prod"}
                 }
               }"#,
        )
        .unwrap()
    }

    fn env_url(url: &str) -> EnvVars {
        EnvVars {
            redis_url: Some(url.into()),
            ..EnvVars::default()
        }
    }

    /// The precedence matrix from ADR-0001, as a table.
    #[test]
    fn precedence_flags_beat_profile_beat_environment_beat_localhost() {
        let cfg = config_with_default();
        let flag_host = Flags {
            host: Some("from-flag".into()),
            ..Flags::default()
        };
        let env = env_url("redis://from-env:6379");

        struct Case<'a> {
            why: &'a str,
            flags: Flags,
            config: Option<&'a Config>,
            env: EnvVars,
            target: &'a str,
            source: Source,
        }

        let cases = vec![
            Case {
                why: "flags outrank everything",
                flags: flag_host.clone(),
                config: Some(&cfg),
                env: env.clone(),
                target: "from-flag:6379/0",
                source: Source::Flag,
            },
            Case {
                why: "default Profile outranks the environment",
                flags: Flags::default(),
                config: Some(&cfg),
                env: env.clone(),
                target: "cache-01:6379/0",
                source: Source::Profile("staging".into()),
            },
            Case {
                why: "environment is used when no config exists",
                flags: Flags::default(),
                config: None,
                env: env.clone(),
                target: "redis://from-env:6379/0",
                source: Source::Environment,
            },
            Case {
                why: "localhost when nothing at all is configured",
                flags: Flags::default(),
                config: None,
                env: EnvVars::default(),
                target: "127.0.0.1:6379/0",
                source: Source::Default,
            },
            Case {
                why: "--profile selects a non-default Profile",
                flags: Flags {
                    profile: Some("prod".into()),
                    ..Flags::default()
                },
                config: Some(&cfg),
                env: env.clone(),
                target: "redis.prod:6380/2",
                source: Source::Profile("prod".into()),
            },
        ];

        for c in cases {
            let got = resolved(&c.flags, c.config, &c.env).connection;
            assert_eq!(got.target, c.target, "{}", c.why);
            assert_eq!(got.source, c.source, "{}", c.why);
        }
    }

    #[test]
    fn redis_url_is_used_wholesale_and_never_merged_with_the_discrete_variables() {
        // A stale REDIS_HOST export must not contaminate an explicit REDIS_URL.
        let env = EnvVars {
            redis_url: Some("redis://real-target:6379".into()),
            redis_host: Some("stale-leftover".into()),
            redis_port: Some("9999".into()),
            ..EnvVars::default()
        };
        let got = resolved(&Flags::default(), None, &env).connection;
        assert_eq!(got.target, "redis://real-target:6379/0");
        assert!(!got.target.contains("stale-leftover"));
        assert!(!got.target.contains("9999"));
    }

    #[test]
    fn the_discrete_variables_are_used_when_there_is_no_redis_url() {
        let env = EnvVars {
            redis_host: Some("box".into()),
            redis_port: Some("6390".into()),
            ..EnvVars::default()
        };
        assert_eq!(
            resolved(&Flags::default(), None, &env).connection.target,
            "box:6390/0"
        );
    }

    #[test]
    fn resolution_never_prompts_and_always_produces_a_target() {
        // Every combination resolves to something. There is no "ask the user".
        for config in [None, Some(&config_with_default())] {
            for env in [EnvVars::default(), env_url("redis://x:1")] {
                let got = resolved(&Flags::default(), config, &env).connection;
                assert!(!got.target.is_empty());
            }
        }
    }

    #[test]
    fn an_untagged_remote_target_is_unknown_not_local() {
        let flags = Flags {
            host: Some("10.0.0.9".into()),
            ..Flags::default()
        };
        assert_eq!(
            resolved(&flags, None, &EnvVars::default())
                .connection
                .environment,
            Environment::Unknown
        );
    }

    #[test]
    fn loopback_is_local() {
        for host in ["127.0.0.1", "localhost"] {
            let flags = Flags {
                host: Some(host.into()),
                ..Flags::default()
            };
            assert_eq!(
                resolved(&flags, None, &EnvVars::default())
                    .connection
                    .environment,
                Environment::Local
            );
        }
    }

    /// ADR-0022 (first handled in `docs/plans/m4-cluster-refusal.md`): `infer_environment` must
    /// strip Sentinel and Cluster scheme variants the same way it already
    /// strips `redis://`/`rediss://`, so a loopback target under either
    /// scheme is `Local` like every other loopback target — not `Unknown`
    /// just because of the scheme it arrived under.
    #[test]
    fn a_loopback_sentinel_or_cluster_url_is_local() {
        for url in [
            "redis-sentinel://127.0.0.1:26379",
            "rediss-sentinel://127.0.0.1:26379",
            "redis-cluster://127.0.0.1:30001",
            "rediss-cluster://127.0.0.1:30001",
            "redis-sentinel://localhost:26379",
        ] {
            let flags = Flags {
                url: Some(url.into()),
                ..Flags::default()
            };
            assert_eq!(
                resolved(&flags, None, &EnvVars::default())
                    .connection
                    .environment,
                Environment::Local,
                "{url} should resolve to Local"
            );
        }
    }

    /// The non-loopback counterpart: a Sentinel/Cluster URL to a remote host
    /// stays `Unknown`, same as a plain `redis://` URL would.
    #[test]
    fn a_non_loopback_sentinel_or_cluster_url_is_unknown() {
        for url in [
            "redis-sentinel://sentinel.example.com:26379",
            "redis-cluster://cluster.example.com:30001",
        ] {
            let flags = Flags {
                url: Some(url.into()),
                ..Flags::default()
            };
            assert_eq!(
                resolved(&flags, None, &EnvVars::default())
                    .connection
                    .environment,
                Environment::Unknown,
                "{url} should resolve to Unknown"
            );
        }
    }

    /// Plain schemes must be unaffected by the new stripping logic.
    #[test]
    fn plain_schemes_are_unchanged_by_the_new_scheme_list() {
        let flags = Flags {
            url: Some("redis://127.0.0.1:6379".into()),
            ..Flags::default()
        };
        assert_eq!(
            resolved(&flags, None, &EnvVars::default())
                .connection
                .environment,
            Environment::Local
        );
        let flags = Flags {
            url: Some("rediss://remote.example.com:6379".into()),
            ..Flags::default()
        };
        assert_eq!(
            resolved(&flags, None, &EnvVars::default())
                .connection
                .environment,
            Environment::Unknown
        );
    }

    #[test]
    fn a_profiles_declared_environment_wins_over_inference() {
        // "cache-01" is not loopback, but the Profile says staging, and a
        // declared Environment is a statement of fact, not a guess.
        let cfg = config_with_default();
        assert_eq!(
            resolved(&Flags::default(), Some(&cfg), &EnvVars::default())
                .connection
                .environment,
            Environment::Staging
        );
    }

    // ── #106: one effective database, shown and dialled ────────────────────

    fn url_flags(url: &str, db: Option<u8>) -> Flags {
        Flags {
            url: Some(url.into()),
            db,
            ..Flags::default()
        }
    }

    /// (display target, dial URL) for a `--url` target.
    fn shown_and_dialled(url: &str, db: Option<u8>) -> (String, String) {
        let r = resolved(&url_flags(url, db), None, &EnvVars::default());
        (r.connection.target, r.dial_url)
    }

    #[test]
    fn a_urls_database_is_replaced_not_appended_and_shown_as_dialled() {
        // (url, --db, displayed, dialled)
        let cases: &[(&str, Option<u8>, &str, &str)] = &[
            // no database in the URL
            ("redis://h:6379", None, "redis://h:6379/0", "redis://h:6379"),
            (
                "redis://h:6379/",
                None,
                "redis://h:6379/0",
                "redis://h:6379/",
            ),
            (
                "redis://h:6379",
                Some(3),
                "redis://h:6379/3",
                "redis://h:6379/3",
            ),
            (
                "redis://h:6379/",
                Some(3),
                "redis://h:6379/3",
                "redis://h:6379/3",
            ),
            (
                "redis://h:6379",
                Some(0),
                "redis://h:6379/0",
                "redis://h:6379",
            ),
            // a database in the URL
            (
                "redis://h:6379/2",
                None,
                "redis://h:6379/2",
                "redis://h:6379/2",
            ),
            (
                "redis://h:6379/2",
                Some(3),
                "redis://h:6379/3",
                "redis://h:6379/3",
            ),
            (
                "redis://h:6379/2",
                Some(2),
                "redis://h:6379/2",
                "redis://h:6379/2",
            ),
            (
                "redis://h:6379/2",
                Some(0),
                "redis://h:6379/0",
                "redis://h:6379/0",
            ),
            // a query string and a sentinel host list survive
            (
                "redis-sentinel://h1:26379,h2:26379/0?sentinelServiceName=x",
                Some(4),
                "redis-sentinel://h1:26379,h2:26379/4?sentinelServiceName=x",
                "redis-sentinel://h1:26379,h2:26379/4?sentinelServiceName=x",
            ),
            (
                "redis-sentinel://h1:26379?sentinelServiceName=x",
                Some(4),
                "redis-sentinel://h1:26379/4?sentinelServiceName=x",
                "redis-sentinel://h1:26379/4?sentinelServiceName=x",
            ),
            (
                "redis-sentinel://h1:26379?sentinelServiceName=x",
                None,
                "redis-sentinel://h1:26379/0?sentinelServiceName=x",
                "redis-sentinel://h1:26379?sentinelServiceName=x",
            ),
            (
                "rediss://h:6380/5?x=1#frag",
                Some(6),
                "rediss://h:6380/6?x=1#frag",
                "rediss://h:6380/6?x=1#frag",
            ),
        ];
        for (url, db, shown, dialled) in cases {
            let (got_shown, got_dial) = shown_and_dialled(url, *db);
            assert_eq!(&got_shown, shown, "display of {url} with --db {db:?}");
            assert_eq!(&got_dial, dialled, "dial of {url} with --db {db:?}");
        }
    }

    #[test]
    fn userinfo_is_redacted_for_display_and_kept_for_dialling() {
        let (shown, dial) = shown_and_dialled("rediss://alice:s3cret@h:6380/2", Some(7));
        assert_eq!(shown, "rediss://alice:•••@h:6380/7");
        assert_eq!(dial, "rediss://alice:s3cret@h:6380/7");
        let (shown, dial) = shown_and_dialled("redis://:s3cret@h:6379", None);
        assert_eq!(shown, "redis://•••@h:6379/0");
        assert_eq!(dial, "redis://:s3cret@h:6379");
        assert!(!shown.contains("s3cret"));
    }

    #[test]
    fn redis_url_in_the_environment_takes_db_too() {
        let flags = Flags {
            db: Some(3),
            ..Flags::default()
        };
        let r = resolved(&flags, None, &env_url("redis://h:6379/2"));
        assert_eq!(r.connection.target, "redis://h:6379/3");
        assert_eq!(r.dial_url, "redis://h:6379/3");
        assert_eq!(r.connection.source, Source::Environment);
        let r = resolved(&Flags::default(), None, &env_url("redis://h:6379/2"));
        assert_eq!(r.connection.target, "redis://h:6379/2");
        assert_eq!(r.dial_url, "redis://h:6379/2");
    }

    #[test]
    fn a_profile_url_shows_what_it_dials_and_db_follows_the_precedence() {
        let cfg = crate::config::parse(
            r#"{"profiles": {
                 "bare":  {"url":"redis://h:6379"},
                 "named": {"url":"redis://h:6379/2"},
                 "both":  {"url":"redis://h:6379/2","db":5}
               }}"#,
        )
        .unwrap();
        let by = |name: &str, db: Option<u8>| {
            let flags = Flags {
                profile: Some(name.into()),
                db,
                ..Flags::default()
            };
            let r = resolved(&flags, Some(&cfg), &EnvVars::default());
            (r.connection.target, r.dial_url)
        };
        let s = |a: &str, b: &str| (a.to_string(), b.to_string());
        // No --db: nothing about what is dialled changes.
        assert_eq!(by("bare", None), s("redis://h:6379/0", "redis://h:6379"));
        assert_eq!(by("named", None), s("redis://h:6379/2", "redis://h:6379/2"));
        // The Profile's `db` outranks its URL's database; `--db` outranks both.
        assert_eq!(by("both", None), s("redis://h:6379/5", "redis://h:6379/5"));
        assert_eq!(
            by("both", Some(1)),
            s("redis://h:6379/1", "redis://h:6379/1")
        );
        assert_eq!(
            by("bare", Some(1)),
            s("redis://h:6379/1", "redis://h:6379/1")
        );
    }

    #[test]
    fn a_cluster_url_accepts_database_0_only() {
        for url in [
            "redis-cluster://127.0.0.1:30001",
            "rediss-cluster://c:30001/0",
        ] {
            for db in [None, Some(0)] {
                resolve(&url_flags(url, db), None, &EnvVars::default())
                    .unwrap_or_else(|e| panic!("{url} --db {db:?} is refused: {e}"));
            }
            let err = resolve(&url_flags(url, Some(3)), None, &EnvVars::default())
                .expect_err("a Cluster has no database 3");
            assert!(matches!(err, ResolveError::ClusterDatabase { db: 3, .. }));
            assert!(err.to_string().contains("only database 0"), "{err}");
        }
        // The URL's own database counts, and a password never reaches the message.
        let err = resolve(
            &url_flags("redis-cluster://u:s3cret@c:30001/2", None),
            None,
            &EnvVars::default(),
        )
        .expect_err("the URL names database 2");
        assert!(!err.to_string().contains("s3cret"), "{err}");
        // From the environment and from a Profile as well.
        let flags = Flags {
            db: Some(3),
            ..Flags::default()
        };
        assert!(resolve(&flags, None, &env_url("redis-cluster://c:30001")).is_err());
        let cfg = crate::config::parse(
            r#"{"profiles":{"c":{"url":"redis-cluster://c:30001","env":"prod"}}}"#,
        )
        .unwrap();
        let flags = Flags {
            profile: Some("c".into()),
            db: Some(3),
            ..Flags::default()
        };
        assert!(resolve(&flags, Some(&cfg), &EnvVars::default()).is_err());
    }

    #[test]
    fn with_database_leaves_urls_without_a_path_database_alone() {
        assert_eq!(
            with_database("redis+unix:///tmp/r.sock?x=1", 3),
            "redis+unix:///tmp/r.sock?x=1"
        );
        assert_eq!(with_database("no-scheme", 3), "no-scheme");
        assert_eq!(url_database("redis://h:1/12"), Some(12));
        assert_eq!(url_database("redis://h:1/"), None);
        assert_eq!(url_database("redis://h:1?a=/9"), None);
    }

    fn unknown(flags: &Flags, config: Option<&Config>) -> ResolveError {
        resolve(flags, config, &EnvVars::default()).expect_err("an unknown Profile is refused")
    }

    #[test]
    fn a_misspelled_profile_flag_is_an_error_listing_the_available_ones() {
        let cfg = config_with_default();
        let flags = Flags {
            profile: Some("stagin".into()),
            ..Flags::default()
        };
        let err = unknown(&flags, Some(&cfg));
        assert_eq!(
            err.to_string(),
            "no Profile named \"stagin\"; available Profiles: prod, staging"
        );
    }

    /// The flag and the positional name arrive as the same `Flags::profile`
    /// (the shell merges them), so one case covers both; `tests/startup.rs`
    /// in the app crate runs both spellings through the binary.
    #[test]
    fn a_misspelled_profile_is_refused_even_beside_an_explicit_target() {
        let cfg = config_with_default();
        let flags = Flags {
            profile: Some("nope".into()),
            host: Some("somewhere".into()),
            ..Flags::default()
        };
        assert!(matches!(
            unknown(&flags, Some(&cfg)),
            ResolveError::UnknownProfile { .. }
        ));
    }

    #[test]
    fn a_profile_name_with_no_config_file_says_so() {
        let flags = Flags {
            profile: Some("prod".into()),
            ..Flags::default()
        };
        let err = unknown(&flags, None);
        assert_eq!(
            err.to_string(),
            "no Profile named \"prod\": there is no config file, so no Profiles are defined"
        );
    }

    #[test]
    fn a_config_with_no_profiles_says_so() {
        let cfg = crate::config::parse(r#"{"profiles":{}}"#).unwrap();
        let flags = Flags {
            profile: Some("prod".into()),
            ..Flags::default()
        };
        assert_eq!(
            unknown(&flags, Some(&cfg)).to_string(),
            "no Profile named \"prod\": the config file defines no Profiles"
        );
    }

    #[test]
    fn a_correctly_spelled_profile_still_resolves() {
        let cfg = config_with_default();
        let flags = Flags {
            profile: Some("prod".into()),
            ..Flags::default()
        };
        let got = resolve(&flags, Some(&cfg), &EnvVars::default()).unwrap();
        assert_eq!(got.connection.source, Source::Profile("prod".into()));
    }

    #[test]
    fn a_db_flag_overrides_the_profiles_database() {
        let cfg = config_with_default();
        let flags = Flags {
            profile: Some("prod".into()),
            db: Some(7),
            ..Flags::default()
        };
        assert_eq!(
            resolved(&flags, Some(&cfg), &EnvVars::default())
                .connection
                .target,
            "redis.prod:6380/7"
        );
    }
}

#[cfg(test)]
mod credential_tests {
    //! Secrets are references, and the reference has to actually reach the
    //! connection — parsing it and then dropping it is worse than not
    //! supporting it, because the config file looks like it works.

    use super::*;

    /// `resolve`, for cases where the Profile is known to exist.
    fn resolved(flags: &Flags, config: Option<&Config>, env: &EnvVars) -> Resolution {
        resolve(flags, config, env).expect("resolves")
    }

    fn config(json: &str) -> Config {
        crate::config::parse(json).unwrap()
    }

    fn creds_for(json: &str, profile: &str) -> Credentials {
        let cfg = config(json);
        let flags = Flags {
            profile: Some(profile.into()),
            ..Flags::default()
        };
        resolved(&flags, Some(&cfg), &EnvVars::default()).credentials
    }

    #[test]
    fn password_env_survives_resolution_as_a_reference() {
        let c = creds_for(
            r#"{"profiles":{"p":{"host":"h","passwordEnv":"REDIS_PW"}}}"#,
            "p",
        );
        assert_eq!(c.password, PasswordSource::Env("REDIS_PW".into()));
    }

    #[test]
    fn password_command_survives_resolution() {
        let c = creds_for(
            r#"{"profiles":{"p":{"host":"h","passwordCommand":"pass show redis"}}}"#,
            "p",
        );
        assert_eq!(
            c.password,
            PasswordSource::Command("pass show redis".into())
        );
    }

    #[test]
    fn a_reference_wins_over_a_literal_in_the_same_profile() {
        // Otherwise adding the recommended form to a Profile that already has a
        // literal would silently change nothing.
        let c = creds_for(
            r#"{"profiles":{"p":{"host":"h","password":"hunter2","passwordEnv":"REDIS_PW"}}}"#,
            "p",
        );
        assert_eq!(c.password, PasswordSource::Env("REDIS_PW".into()));
    }

    #[test]
    fn a_username_and_tls_flag_reach_the_connection_too() {
        let c = creds_for(
            r#"{"profiles":{"p":{"host":"h","username":"app","tls":true}}}"#,
            "p",
        );
        assert_eq!(c.username.as_deref(), Some("app"));
        assert!(c.tls);
    }

    #[test]
    fn a_profile_with_no_credentials_asks_for_none() {
        let c = creds_for(r#"{"profiles":{"p":{"host":"h"}}}"#, "p");
        assert_eq!(c.password, PasswordSource::None);
        assert!(c.username.is_none());
    }

    #[test]
    fn the_environment_supplies_credentials_when_the_target_came_from_there() {
        let env = EnvVars {
            redis_url: Some("redis://box:6379".into()),
            redis_user: Some("app".into()),
            redis_password: Some("s3cret".into()),
            ..EnvVars::default()
        };
        let c = resolved(&Flags::default(), None, &env).credentials;
        assert_eq!(c.username.as_deref(), Some("app"));
        assert_eq!(c.password, PasswordSource::Literal("s3cret".into()));
    }

    #[test]
    fn a_profiles_credentials_are_not_merged_with_the_environments() {
        // ADR-0001 forbids component-level merging: a stale REDIS_PASSWORD must
        // not attach itself to a Profile that states its own credentials.
        let cfg = config(r#"{"profiles":{"p":{"host":"h","passwordEnv":"PROFILE_PW"}}}"#);
        let env = EnvVars {
            redis_password: Some("stale".into()),
            redis_user: Some("stale-user".into()),
            ..EnvVars::default()
        };
        let flags = Flags {
            profile: Some("p".into()),
            ..Flags::default()
        };
        let c = resolved(&flags, Some(&cfg), &env).credentials;
        assert_eq!(c.password, PasswordSource::Env("PROFILE_PW".into()));
        assert!(c.username.is_none(), "the stale username must not leak in");
    }

    #[test]
    fn a_password_flag_overrides_a_profiles_stored_password() {
        // `redis-pane --profile prod --password "$(vault read ...)"` has to
        // actually reach the connection, the same way an explicit --host beats
        // a Profile's target (ADR-0001) — the flag is the most explicit thing
        // on the command line, so it wins here too.
        let cfg = config(r#"{"profiles":{"p":{"host":"h","passwordEnv":"PROFILE_PW"}}}"#);
        let flags = Flags {
            profile: Some("p".into()),
            password: Some("override-me".into()),
            ..Flags::default()
        };
        let c = resolved(&flags, Some(&cfg), &EnvVars::default()).credentials;
        assert_eq!(c.password, PasswordSource::Literal("override-me".into()));
    }

    #[test]
    fn a_password_flag_applies_even_when_the_target_came_from_host_and_port() {
        // Before this flag existed, --host/--port had no way to carry a
        // password at all except REDIS_PASSWORD — this is the missing case.
        let flags = Flags {
            host: Some("cache-01".into()),
            password: Some("hunter2".into()),
            ..Flags::default()
        };
        let c = resolved(&flags, None, &EnvVars::default()).credentials;
        assert_eq!(c.password, PasswordSource::Literal("hunter2".into()));
    }

    #[test]
    fn a_user_flag_and_tls_flag_are_applied_the_same_way() {
        let flags = Flags {
            host: Some("cache-01".into()),
            user: Some("app".into()),
            tls: true,
            ..Flags::default()
        };
        let c = resolved(&flags, None, &EnvVars::default()).credentials;
        assert_eq!(c.username.as_deref(), Some("app"));
        assert!(c.tls);
    }

    #[test]
    fn no_flag_credentials_leaves_the_existing_source_untouched() {
        // Regression guard: today's REDIS_PASSWORD-with-host/port behavior
        // must survive unchanged when none of the new flags are set.
        let env = EnvVars {
            redis_password: Some("from-env".into()),
            ..EnvVars::default()
        };
        let flags = Flags {
            host: Some("cache-01".into()),
            ..Flags::default()
        };
        let c = resolved(&flags, None, &env).credentials;
        assert_eq!(c.password, PasswordSource::Literal("from-env".into()));
    }
}

#[cfg(test)]
mod redaction_tests {
    //! A password must never reach the title bar. It is shown permanently
    //! (ADR-0001), so anything in it is on screen for the whole session.

    use super::*;

    /// `resolve`, for cases where the Profile is known to exist.
    fn resolved(flags: &Flags, config: Option<&Config>, env: &EnvVars) -> Resolution {
        resolve(flags, config, env).expect("resolves")
    }

    #[test]
    fn a_password_in_a_url_is_replaced() {
        assert_eq!(
            redact("rediss://default:s3cret@cache.example.com:6379"),
            "rediss://default:•••@cache.example.com:6379"
        );
    }

    #[test]
    fn the_username_survives_because_it_is_not_a_secret() {
        // Which ACL user is connected is exactly the kind of fact the Source
        // readout exists to make visible.
        assert!(redact("rediss://app-reader:pw@h:6379").starts_with("rediss://app-reader:"));
    }

    #[test]
    fn a_password_with_no_username_leaves_nothing_behind() {
        assert_eq!(redact("redis://:s3cret@h:6379"), "redis://•••@h:6379");
    }

    #[test]
    fn a_url_with_no_credentials_is_untouched() {
        assert_eq!(redact("redis://cache-01:6379"), "redis://cache-01:6379");
        assert_eq!(redact("cache-01:6379/0"), "cache-01:6379/0");
    }

    #[test]
    fn resolution_never_produces_a_target_containing_a_password() {
        let env = EnvVars {
            redis_url: Some("rediss://default:hunter2@h.upstash.io:6379".into()),
            ..EnvVars::default()
        };
        let target = resolved(&Flags::default(), None, &env).connection.target;
        assert!(
            !target.contains("hunter2"),
            "leaked into the title bar: {target}"
        );

        let flags = Flags {
            url: Some("rediss://default:hunter2@h:6379".into()),
            ..Flags::default()
        };
        let target = resolved(&flags, None, &EnvVars::default())
            .connection
            .target;
        assert!(
            !target.contains("hunter2"),
            "leaked into the title bar: {target}"
        );
    }
}
