//! Config schema and validation — pure (ADR-0002, ADR-0003, PLAN M0.7).
//!
//! Reading the file is a shell concern; describing and validating it is not.
//!
//! **Unknown fields are a parse error, never ignored.** The app only ever reads
//! this file, so every byte in it was typed by hand, which makes a misspelling
//! the likeliest defect — and a misspelled `passwordEnv` is a credential
//! silently dropped. A Profile with no `env` gets `Unknown`, never `Local`
//! (ADR-0004).

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};

use crate::state::Environment;
use crate::theme::{BUILTIN_NAMES, ColorSpec, Palette, Rgb, Token};

/// The whole config file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    /// Which Profile is used when no `--profile` is given.
    pub default_profile: Option<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, Profile>,
    /// `true` draws with the ASCII glyph set, `false` with Unicode; absent
    /// leaves it to the locale (DESIGN §5). `--ascii`/`--unicode` override it.
    pub ascii: Option<bool>,
    /// The theme used when no `--theme` is given: a built-in name or a key of
    /// `themes`. Absent means `dark`.
    ///
    /// Note: like every field here, this is refused by an older binary that
    /// predates it (unknown fields are a parse error), so rolling back past
    /// the version that added `theme`/`themes` means removing them first.
    pub theme: Option<String>,
    /// User themes, by name.
    #[serde(default)]
    pub themes: BTreeMap<String, ThemeDef>,
}

/// A user theme: a token → colour map over a built-in `base`.
///
/// Only the tokens it names are overridden; the rest come from the base. An
/// unknown token name is a parse error, for the reason an unknown field is
/// (a typo'd `border-focus` is a colour silently dropped).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeDef {
    /// A built-in theme to inherit from. Absent means `dark`.
    pub base: Option<String>,
    pub colors: Vec<(Token, ColorSpec)>,
}

impl<'de> Deserialize<'de> for ThemeDef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ThemeDef;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a map of token names to colours")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ThemeDef, A::Error> {
                let mut def = ThemeDef::default();
                while let Some(key) = map.next_key::<String>()? {
                    if key == "base" {
                        def.base = Some(map.next_value::<String>()?);
                        continue;
                    }
                    let Some(token) = Token::from_name(&key) else {
                        return Err(de::Error::custom(unknown_token_message(&key)));
                    };
                    let spec = map
                        .next_value_seed(ColorSpecSeed {
                            allows_terminal: token == Token::Background,
                        })?
                        .0;
                    def.colors.push((token, spec));
                }
                Ok(def)
            }
        }
        d.deserialize_map(V)
    }
}

fn unknown_token_message(key: &str) -> String {
    if key == "open-row" {
        return "theme token `open-row` is an underline, not a colour, and cannot be themed".into();
    }
    let names: Vec<_> = Token::ALL.iter().filter_map(|t| t.name()).collect();
    format!(
        "unknown theme token `{key}` (tokens: base, {})",
        names.join(", ")
    )
}

/// `"#rrggbb"` or `{ "fg": "#rrggbb", "bg": "#rrggbb" }` — and, for the
/// `background` token alone, `"terminal"` (paint nothing).
struct ColorSpecDef(ColorSpec);

/// Deserialises one token's colour, knowing which token it is for: `"terminal"`
/// is a value of `background` and a mistake anywhere else.
struct ColorSpecSeed {
    allows_terminal: bool,
}

impl<'de> de::DeserializeSeed<'de> for ColorSpecSeed {
    type Value = ColorSpecDef;
    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<ColorSpecDef, D::Error> {
        struct V {
            allows_terminal: bool,
        }
        fn hex<E: de::Error>(s: &str) -> Result<Rgb, E> {
            Rgb::parse(s).ok_or_else(|| E::custom(format!("`{s}` is not a colour (use #rrggbb)")))
        }
        impl<'de> Visitor<'de> for V {
            type Value = ColorSpecDef;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a \"#rrggbb\" colour, or { \"fg\": .., \"bg\": .. }")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ColorSpecDef, E> {
                if v == "terminal" {
                    return if self.allows_terminal {
                        Ok(ColorSpecDef(ColorSpec::Terminal))
                    } else {
                        Err(E::custom(
                            "`terminal` is a value of `background` only (use #rrggbb)",
                        ))
                    };
                }
                if self.allows_terminal {
                    return Rgb::parse(v)
                        .map(|c| ColorSpecDef(ColorSpec::Primary(c)))
                        .ok_or_else(|| {
                            E::custom(format!(
                                "`{v}` is not a colour (use #rrggbb, or \"terminal\" to paint nothing)"
                            ))
                        });
                }
                Ok(ColorSpecDef(ColorSpec::Primary(hex(v)?)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ColorSpecDef, A::Error> {
                let (mut fg, mut bg) = (None, None);
                while let Some(key) = map.next_key::<String>()? {
                    let value = hex(&map.next_value::<String>()?)?;
                    match key.as_str() {
                        "fg" => fg = Some(value),
                        "bg" => bg = Some(value),
                        other => {
                            return Err(de::Error::custom(format!(
                                "unknown colour channel `{other}` (expected fg or bg)"
                            )));
                        }
                    }
                }
                Ok(ColorSpecDef(ColorSpec::Channels { fg, bg }))
            }
        }
        d.deserialize_any(V {
            allows_terminal: self.allows_terminal,
        })
    }
}

/// A named target the user has written down. A Profile is not a Connection: it
/// is the description, not the live thing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Profile {
    /// Used wholesale when present; never merged with `host`/`port`.
    pub url: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub db: Option<u8>,
    pub username: Option<String>,
    /// A literal password. Honoured, but the file is refused when it is group-
    /// or world-readable, and such Profiles are badged in the UI.
    pub password: Option<String>,
    /// Names an environment variable holding the password. Preferred.
    pub password_env: Option<String>,
    /// A command run at connect time whose stdout is the password. Preferred.
    pub password_command: Option<String>,
    pub tls: Option<bool>,
    /// JSON has no comments, so this stands in for one — and is rendered next
    /// to the Connection, which is better than a comment for the purpose
    /// people would use one (ADR-0002).
    pub note: Option<String>,
    /// Absent means [`Environment::Unknown`], which starts in Read-only Mode.
    pub env: Option<Environment>,
}

impl Profile {
    /// The Environment this Profile declares. Untagged means `unknown`, which
    /// is a real Environment and not a synonym for `local` (ADR-0004).
    pub fn environment(&self) -> Environment {
        self.env.unwrap_or(Environment::Unknown)
    }

    /// Whether this Profile carries a literal password rather than a reference.
    pub fn has_literal_password(&self) -> bool {
        self.password.is_some()
    }
}

/// Why a config file was rejected. Every variant names the file so the message
/// can be acted on without guessing (R1.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// Malformed JSON, or a field the app does not recognise.
    Parse {
        line: usize,
        column: usize,
        detail: String,
    },
    /// `defaultProfile` names a Profile that is not in the file.
    UnknownDefaultProfile(String),
    /// `theme` names neither a built-in nor an entry in `themes`.
    UnknownTheme(String),
    /// A user theme's `base` is not a built-in.
    UnknownThemeBase { theme: String, base: String },
    /// A user theme is named like a built-in, so `--theme dark` would be
    /// ambiguous.
    ThemeShadowsBuiltin(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Parse {
                line,
                column,
                detail,
            } => {
                write!(f, "line {line}, column {column}: {detail}")
            }
            ConfigError::UnknownDefaultProfile(name) => {
                write!(f, "defaultProfile names \"{name}\", which is not defined")
            }
            ConfigError::UnknownTheme(name) => {
                write!(
                    f,
                    "theme names \"{name}\", which is neither a built-in ({}) nor defined in themes",
                    BUILTIN_NAMES.join(", ")
                )
            }
            ConfigError::UnknownThemeBase { theme, base } => {
                write!(
                    f,
                    "theme \"{theme}\" has base \"{base}\", which is not a built-in ({})",
                    BUILTIN_NAMES.join(", ")
                )
            }
            ConfigError::ThemeShadowsBuiltin(name) => {
                write!(
                    f,
                    "themes defines \"{name}\", which is a built-in theme's name"
                )
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Parse and validate a config file's contents.
///
/// Pure: takes the text, returns a `Config` or a located error. Reading the
/// file, and refusing it for being too readable, belong to the shell.
pub fn parse(text: &str) -> Result<Config, ConfigError> {
    let config: Config = serde_json::from_str(text).map_err(|e| ConfigError::Parse {
        line: e.line(),
        column: e.column(),
        // serde_json appends its own " at line N column M"; we print the
        // location ourselves, so saying it twice just makes the message harder
        // to read at the moment someone is trying to fix a typo.
        detail: strip_location(&e.to_string()),
    })?;

    if let Some(name) = &config.default_profile
        && !config.profiles.contains_key(name)
    {
        return Err(ConfigError::UnknownDefaultProfile(name.clone()));
    }
    for (name, def) in &config.themes {
        if Palette::builtin(name).is_some() {
            return Err(ConfigError::ThemeShadowsBuiltin(name.clone()));
        }
        if let Some(base) = &def.base
            && Palette::builtin(base).is_none()
        {
            return Err(ConfigError::UnknownThemeBase {
                theme: name.clone(),
                base: base.clone(),
            });
        }
    }
    if let Some(name) = &config.theme
        && Palette::builtin(name).is_none()
        && !config.themes.contains_key(name)
    {
        return Err(ConfigError::UnknownTheme(name.clone()));
    }
    Ok(config)
}

fn strip_location(message: &str) -> String {
    match message.rfind(" at line ") {
        Some(i) => message[..i].to_string(),
        None => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_is_an_optional_top_level_boolean() {
        assert_eq!(parse("{}").unwrap().ascii, None);
        assert_eq!(parse(r#"{"ascii":true}"#).unwrap().ascii, Some(true));
        assert_eq!(parse(r#"{"ascii":false}"#).unwrap().ascii, Some(false));
        assert!(
            parse(r#"{"ascii":"yes"}"#).is_err(),
            "a string is not a bool"
        );
        assert!(
            parse(r#"{"asci":true}"#).is_err(),
            "a misspelling is an error"
        );
    }

    #[test]
    fn a_minimal_file_parses() {
        let c =
            parse(r#"{"profiles":{"staging":{"host":"cache-01","port":6379,"env":"staging"}}}"#)
                .unwrap();
        let p = &c.profiles["staging"];
        assert_eq!(p.host.as_deref(), Some("cache-01"));
        assert_eq!(p.environment(), Environment::Staging);
    }

    #[test]
    fn an_empty_object_is_valid() {
        assert_eq!(parse("{}").unwrap(), Config::default());
    }

    #[test]
    fn a_profile_without_env_is_unknown_never_local() {
        let c = parse(r#"{"profiles":{"box":{"host":"10.0.0.9"}}}"#).unwrap();
        assert_eq!(c.profiles["box"].environment(), Environment::Unknown);
    }

    #[test]
    fn a_misspelled_password_env_is_refused_not_ignored() {
        // The whole point of deny_unknown_fields: this typo would otherwise
        // drop the credential silently.
        let err = parse(r#"{"profiles":{"p":{"passwordEnvironment":"REDIS_PW"}}}"#).unwrap_err();
        match err {
            ConfigError::Parse { detail, .. } => {
                assert!(detail.contains("passwordEnvironment"), "got: {detail}");
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_parse_error_carries_a_line_and_column() {
        let err = parse("{\n  \"profiles\": {\n    oops\n  }\n}").unwrap_err();
        match err {
            ConfigError::Parse { line, column, .. } => {
                assert_eq!(line, 3);
                assert!(column > 0);
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn the_location_is_stated_once_not_twice() {
        let err = parse(r#"{"profiles":{"p":{"nope":1}}}"#).unwrap_err();
        let msg = err.to_string();
        assert_eq!(msg.matches("line ").count(), 1, "got: {msg}");
    }

    #[test]
    fn default_profile_must_exist() {
        let err = parse(r#"{"defaultProfile":"nope","profiles":{"p":{}}}"#).unwrap_err();
        assert_eq!(err, ConfigError::UnknownDefaultProfile("nope".into()));
    }

    #[test]
    fn every_environment_name_round_trips() {
        for (text, expected) in [
            ("local", Environment::Local),
            ("staging", Environment::Staging),
            ("prod", Environment::Prod),
            ("unknown", Environment::Unknown),
        ] {
            let c = parse(&format!(r#"{{"profiles":{{"p":{{"env":"{text}"}}}}}}"#)).unwrap();
            assert_eq!(c.profiles["p"].environment(), expected);
        }
    }

    #[test]
    fn an_unrecognised_environment_is_refused() {
        assert!(parse(r#"{"profiles":{"p":{"env":"production"}}}"#).is_err());
    }

    #[test]
    fn a_literal_password_is_detected_so_the_ui_can_badge_it() {
        let c = parse(r#"{"profiles":{"p":{"password":"hunter2"}}}"#).unwrap();
        assert!(c.profiles["p"].has_literal_password());
    }

    #[test]
    fn theme_and_themes_parse_and_default_to_absent() {
        let c = parse(
            r##"{"theme":"mine","themes":{"mine":{"base":"light","border-focus":"#112233"}}}"##,
        )
        .unwrap();
        assert_eq!(c.theme.as_deref(), Some("mine"));
        let def = &c.themes["mine"];
        assert_eq!(def.base.as_deref(), Some("light"));
        assert_eq!(
            def.colors,
            vec![(
                Token::BorderFocus,
                ColorSpec::Primary(Rgb(0x11, 0x22, 0x33))
            )]
        );
        assert_eq!(parse("{}").unwrap().theme, None);
    }

    #[test]
    fn a_misspelled_theme_token_is_refused_and_named() {
        let err = parse(r##"{"themes":{"mine":{"bordr-focus":"#112233"}}}"##).unwrap_err();
        match err {
            ConfigError::Parse { detail, .. } => {
                assert!(detail.contains("bordr-focus"), "got: {detail}");
                assert!(
                    detail.contains("border-focus"),
                    "lists valid tokens: {detail}"
                );
            }
            other => panic!("expected a parse error, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_colour_is_refused() {
        for bad in ["red", "#fff", "#12345g", "112233"] {
            let text = format!(r#"{{"themes":{{"m":{{"text":"{bad}"}}}}}}"#);
            assert!(parse(&text).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn selected_takes_both_channels() {
        let c =
            parse(r##"{"themes":{"m":{"selected":{"fg":"#000000","bg":"#ffffff"}}}}"##).unwrap();
        assert_eq!(
            c.themes["m"].colors[0].1,
            ColorSpec::Channels {
                fg: Some(Rgb(0, 0, 0)),
                bg: Some(Rgb(255, 255, 255))
            }
        );
    }

    #[test]
    fn background_is_a_colour_or_terminal_and_nothing_else() {
        let c = parse(r##"{"themes":{"m":{"background":"#fafafa"}}}"##).unwrap();
        assert_eq!(
            c.themes["m"].colors,
            vec![(Token::Background, ColorSpec::Primary(Rgb(0xFA, 0xFA, 0xFA)))]
        );
        let c = parse(r#"{"themes":{"m":{"base":"light","background":"terminal"}}}"#).unwrap();
        assert_eq!(
            c.themes["m"].colors,
            vec![(Token::Background, ColorSpec::Terminal)]
        );
    }

    #[test]
    fn a_typod_background_is_a_parse_error_with_a_position() {
        for bad in ["terminl", "white", "#fff", "Terminal"] {
            let text = format!(
                "{{\n  \"themes\": {{\n    \"m\": {{ \"background\": \"{bad}\" }}\n  }}\n}}"
            );
            match parse(&text).unwrap_err() {
                ConfigError::Parse {
                    line,
                    column,
                    detail,
                    ..
                } => {
                    assert_eq!(line, 3, "{bad}");
                    assert!(column > 0, "{bad}");
                    assert!(detail.contains(bad), "{detail}");
                }
                other => panic!("{bad}: expected a parse error, got {other:?}"),
            }
        }
    }

    #[test]
    fn terminal_is_refused_on_every_other_token() {
        let err = parse(r#"{"themes":{"m":{"text":"terminal"}}}"#).unwrap_err();
        assert!(err.to_string().contains("background"), "{err}");
    }

    #[test]
    fn open_row_cannot_be_themed() {
        let err = parse(r##"{"themes":{"m":{"open-row":"#ffffff"}}}"##).unwrap_err();
        assert!(err.to_string().contains("open-row"), "{err}");
    }

    #[test]
    fn the_theme_must_exist() {
        assert_eq!(
            parse(r#"{"theme":"nope"}"#).unwrap_err(),
            ConfigError::UnknownTheme("nope".into())
        );
        assert!(parse(r#"{"theme":"light"}"#).is_ok());
    }

    #[test]
    fn a_base_must_be_a_builtin_and_a_theme_may_not_shadow_one() {
        assert_eq!(
            parse(r#"{"themes":{"m":{"base":"other"}}}"#).unwrap_err(),
            ConfigError::UnknownThemeBase {
                theme: "m".into(),
                base: "other".into()
            }
        );
        assert_eq!(
            parse(r#"{"themes":{"dark":{}}}"#).unwrap_err(),
            ConfigError::ThemeShadowsBuiltin("dark".into())
        );
    }
}
