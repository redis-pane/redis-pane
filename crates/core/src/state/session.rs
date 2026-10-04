//! Session restore: what a relaunch brings back, and the file format it
//! travels in (M4 task 7, `docs/plans/m4-session-restore.md`, ADR-0003).
//!
//! Everything here is pure. [`SessionState`] is a snapshot of five things —
//! pane split, tree/flat, sort, filter and the selected key — and
//! [`SessionFile`] is the parsed form of `$XDG_STATE_HOME/redis-pane/state.json`,
//! keyed per target. Reading and writing that file is the shell's job
//! (`crates/app/src/state_file.rs`); this module only turns text into state
//! and back, so a corrupt or old file is a `Result`, never a crash.
//!
//! **No secrets.** The key under which a session is stored is the display
//! target (`host:port/db`), which is already redacted of credentials; nothing
//! here ever sees a password (ADR-0003, R1.6).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{FilterMode, SortBy, State};

/// The only file format this binary understands. A file carrying any other
/// version is ignored with a notice rather than half-read (decision 5).
pub const SESSION_VERSION: u32 = 1;

/// How many targets the file remembers. The oldest by `saved_at_ms` is
/// evicted past this, so the file cannot grow without bound across years of
/// ad-hoc targets.
pub const MAX_TARGETS: usize = 32;

/// What a relaunch against the same target restores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionState {
    pub split_adjust: i16,
    pub tree_mode: bool,
    pub sort: SortBy,
    pub filter: String,
    pub filter_mode: FilterMode,
    /// The key the cursor was on. Bytes, not text: Redis key names are not
    /// guaranteed UTF-8 (R3.13), so this never passes through a lossy string.
    pub selected_key: Option<Vec<u8>>,
}

impl State {
    /// The part of this session worth bringing back.
    ///
    /// While a restored selection is still waiting for the scan to find its
    /// key, the snapshot carries *that* key rather than whatever row 0 happens
    /// to be: otherwise the first debounced write of a relaunch would
    /// overwrite the remembered key with the top of an unfinished list.
    pub fn session_snapshot(&self) -> SessionState {
        let selected_key = self.restore_key.clone().or_else(|| {
            self.selected_key()
                .and_then(|i| self.keys.name(i))
                .map(<[u8]>::to_vec)
        });
        SessionState {
            split_adjust: self.split_adjust,
            tree_mode: self.tree_mode,
            sort: self.list.sort,
            filter: self.list.filter.clone(),
            filter_mode: self.list.mode,
            selected_key,
        }
    }

    /// Apply a restored session at startup, before the first scan.
    ///
    /// The selected key cannot be located yet — nothing is loaded — so it is
    /// held in `restore_key` and resolved by `scan_batch` when the key arrives
    /// (and abandoned silently if it never does).
    pub fn apply_session(&mut self, restored: SessionState) {
        self.split_adjust = restored.split_adjust;
        self.tree_mode = restored.tree_mode;
        self.list.sort = restored.sort;
        self.list.filter = restored.filter;
        self.list.mode = restored.filter_mode;
        self.restore_key = restored.selected_key;
        // Enforces the tree-mode-implies-name-order invariant too.
        self.rebuild_list();
    }
}

/// Why a state file could not be used. Either way the session proceeds as if
/// there were no file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionFileError {
    /// Not valid JSON, or not the shape this version writes.
    Unreadable(String),
    /// Valid JSON carrying a `version` this binary does not know.
    UnknownVersion(Option<u64>),
}

impl std::fmt::Display for SessionFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionFileError::Unreadable(detail) => write!(f, "unreadable ({detail})"),
            SessionFileError::UnknownVersion(Some(v)) => write!(
                f,
                "format version {v}, but this build reads version {SESSION_VERSION}"
            ),
            SessionFileError::UnknownVersion(None) => {
                write!(
                    f,
                    "no format version; this build reads version {SESSION_VERSION}"
                )
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct FileRepr {
    version: u32,
    targets: BTreeMap<String, EntryRepr>,
}

#[derive(Serialize, Deserialize)]
struct EntryRepr {
    split_adjust: i16,
    tree: bool,
    sort: String,
    filter: String,
    filter_mode: String,
    /// Hex of the key's bytes, so a binary name survives JSON intact.
    selected_key: Option<String>,
    saved_at_ms: u64,
}

/// Every remembered session, keyed by target.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionFile {
    targets: BTreeMap<String, (SessionState, u64)>,
}

impl SessionFile {
    pub fn parse(text: &str) -> Result<Self, SessionFileError> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| SessionFileError::Unreadable(e.to_string()))?;
        match value.get("version").and_then(serde_json::Value::as_u64) {
            Some(v) if v == u64::from(SESSION_VERSION) => {}
            other => return Err(SessionFileError::UnknownVersion(other)),
        }
        let repr: FileRepr = serde_json::from_value(value)
            .map_err(|e| SessionFileError::Unreadable(e.to_string()))?;
        let mut targets = BTreeMap::new();
        for (target, e) in repr.targets {
            let state = SessionState {
                split_adjust: e.split_adjust,
                tree_mode: e.tree,
                sort: sort_from(&e.sort)
                    .ok_or_else(|| SessionFileError::Unreadable(format!("sort {:?}", e.sort)))?,
                filter: e.filter,
                filter_mode: mode_from(&e.filter_mode).ok_or_else(|| {
                    SessionFileError::Unreadable(format!("filter_mode {:?}", e.filter_mode))
                })?,
                selected_key: match e.selected_key {
                    None => None,
                    Some(h) => Some(
                        from_hex(&h)
                            .ok_or_else(|| SessionFileError::Unreadable("selected_key".into()))?,
                    ),
                },
            };
            targets.insert(target, (state, e.saved_at_ms));
        }
        Ok(SessionFile { targets })
    }

    /// Pretty-printed: nobody edits this file, but a human debugging it
    /// should be able to read it.
    pub fn to_json(&self) -> String {
        let repr = FileRepr {
            version: SESSION_VERSION,
            targets: self
                .targets
                .iter()
                .map(|(t, (s, at))| {
                    (
                        t.clone(),
                        EntryRepr {
                            split_adjust: s.split_adjust,
                            tree: s.tree_mode,
                            sort: sort_to(s.sort).into(),
                            filter: s.filter.clone(),
                            filter_mode: mode_to(s.filter_mode).into(),
                            selected_key: s.selected_key.as_deref().map(to_hex),
                            saved_at_ms: *at,
                        },
                    )
                })
                .collect(),
        };
        serde_json::to_string_pretty(&repr).unwrap_or_default()
    }

    pub fn get(&self, target: &str) -> Option<&SessionState> {
        self.targets.get(target).map(|(s, _)| s)
    }

    pub fn len(&self) -> usize {
        self.targets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.targets.is_empty()
    }

    /// Remember a session for a target, evicting the least recently saved
    /// target if the file is over [`MAX_TARGETS`].
    pub fn put(&mut self, target: &str, state: SessionState, now_ms: u64) {
        self.targets.insert(target.to_owned(), (state, now_ms));
        while self.targets.len() > MAX_TARGETS {
            let oldest = self
                .targets
                .iter()
                .min_by_key(|(_, (_, at))| *at)
                .map(|(t, _)| t.clone());
            match oldest {
                Some(t) => self.targets.remove(&t),
                None => break,
            };
        }
    }
}

fn sort_to(s: SortBy) -> &'static str {
    match s {
        SortBy::Scan => "scan",
        SortBy::Name => "name",
        SortBy::Ttl => "ttl",
        SortBy::Size => "size",
        SortBy::Kind => "kind",
    }
}

fn sort_from(s: &str) -> Option<SortBy> {
    Some(match s {
        "scan" => SortBy::Scan,
        "name" => SortBy::Name,
        "ttl" => SortBy::Ttl,
        "size" => SortBy::Size,
        "kind" => SortBy::Kind,
        _ => return None,
    })
}

fn mode_to(m: FilterMode) -> &'static str {
    match m {
        FilterMode::Glob => "glob",
        FilterMode::Fuzzy => "fuzzy",
    }
}

fn mode_from(s: &str) -> Option<FilterMode> {
    match s {
        "glob" => Some(FilterMode::Glob),
        "fuzzy" => Some(FilterMode::Fuzzy),
        _ => None,
    }
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::Msg;
    use crate::update::update;

    fn sample() -> SessionState {
        SessionState {
            split_adjust: -7,
            tree_mode: false,
            sort: SortBy::Size,
            filter: "user:*".into(),
            filter_mode: FilterMode::Fuzzy,
            selected_key: Some(b"user:2".to_vec()),
        }
    }

    fn scanned(keys: &[&[u8]], restored: SessionState) -> State {
        let mut s = State {
            rows: 30,
            cols: 120,
            ..State::default()
        };
        s.apply_session(restored);
        let (s, _) = update(
            s,
            Msg::ScanStarted {
                estimated_total: 10,
            },
        );
        let (s, _) = update(
            s,
            Msg::ScanBatch {
                keys: keys.iter().map(|k| k.to_vec()).collect(),
            },
        );
        s
    }

    #[test]
    fn every_field_round_trips_through_file_text() {
        let mut file = SessionFile::default();
        file.put("127.0.0.1:6379/0", sample(), 5);
        let back = SessionFile::parse(&file.to_json()).unwrap();
        assert_eq!(back, file);
        assert_eq!(back.get("127.0.0.1:6379/0"), Some(&sample()));
    }

    #[test]
    fn a_binary_key_name_survives_byte_for_byte() {
        let mut s = sample();
        s.selected_key = Some(vec![0xff, 0x00, b'a', 0xc3, 0x28]);
        let mut file = SessionFile::default();
        file.put("t", s.clone(), 1);
        let back = SessionFile::parse(&file.to_json()).unwrap();
        assert_eq!(back.get("t"), Some(&s));
    }

    #[test]
    fn a_session_is_only_found_under_its_own_target() {
        let mut file = SessionFile::default();
        file.put("staging:6379/0", sample(), 1);
        assert!(file.get("prod:6379/0").is_none());
    }

    #[test]
    fn garbage_and_unknown_versions_are_errors_not_panics() {
        assert!(matches!(
            SessionFile::parse("{not json"),
            Err(SessionFileError::Unreadable(_))
        ));
        assert_eq!(
            SessionFile::parse(r#"{"version": 99, "targets": {}}"#),
            Err(SessionFileError::UnknownVersion(Some(99)))
        );
        assert_eq!(
            SessionFile::parse(r#"{"targets": {}}"#),
            Err(SessionFileError::UnknownVersion(None))
        );
        assert!(matches!(
            SessionFile::parse(r#"{"version": 1, "targets": {"t": {"nope": 1}}}"#),
            Err(SessionFileError::Unreadable(_))
        ));
        assert!(matches!(
            SessionFile::parse(
                r#"{"version":1,"targets":{"t":{"split_adjust":0,"tree":true,"sort":"zzz",
                "filter":"","filter_mode":"glob","selected_key":null,"saved_at_ms":0}}}"#
            ),
            Err(SessionFileError::Unreadable(_))
        ));
    }

    #[test]
    fn the_oldest_target_is_evicted_past_the_cap() {
        let mut file = SessionFile::default();
        for i in 0..=MAX_TARGETS {
            file.put(&format!("t{i}"), sample(), i as u64);
        }
        assert_eq!(file.len(), MAX_TARGETS);
        assert!(file.get("t0").is_none());
        assert!(file.get("t1").is_some());
    }

    #[test]
    fn snapshot_and_apply_round_trip_the_view_fields() {
        let mut s = State::default();
        let mut restored = sample();
        restored.selected_key = None;
        s.apply_session(restored.clone());
        assert_eq!(s.session_snapshot(), restored);
    }

    #[test]
    fn tree_mode_restores_with_name_order() {
        let mut restored = sample();
        restored.tree_mode = true;
        restored.sort = SortBy::Size;
        let mut s = State::default();
        s.apply_session(restored);
        assert!(s.tree_mode);
        assert_eq!(s.list.sort, SortBy::Name, "tree mode folds in name order");
    }

    #[test]
    fn the_selected_key_is_found_when_the_scan_delivers_it() {
        let mut restored = sample();
        restored.filter.clear();
        restored.sort = SortBy::Scan;
        restored.tree_mode = false;
        restored.selected_key = Some(b"k:3".to_vec());
        let s = scanned(&[b"k:1", b"k:2", b"k:3", b"k:4"], restored);
        let i = s.selected_key().unwrap();
        assert_eq!(s.keys.name(i).unwrap(), b"k:3");
        assert!(s.restore_key.is_none());
    }

    #[test]
    fn a_selected_key_that_never_turns_up_leaves_the_natural_selection() {
        let mut restored = sample();
        restored.filter.clear();
        restored.sort = SortBy::Scan;
        restored.tree_mode = false;
        restored.selected_key = Some(b"gone".to_vec());
        let s = scanned(&[b"k:1", b"k:2"], restored);
        assert_eq!(s.view.selected, 0);
        let (s, _) = update(s, Msg::ScanComplete);
        assert!(s.restore_key.is_none(), "given up once the scan ends");
        assert_eq!(s.view.selected, 0);
    }

    #[test]
    fn a_pending_selection_is_not_overwritten_in_the_snapshot() {
        let mut restored = sample();
        restored.selected_key = Some(b"later".to_vec());
        let mut s = State::default();
        s.apply_session(restored);
        assert_eq!(s.session_snapshot().selected_key, Some(b"later".to_vec()));
    }

    #[test]
    fn restoring_works_in_a_sorted_tree_view_too() {
        let mut restored = sample();
        restored.filter.clear();
        restored.tree_mode = true;
        restored.selected_key = Some(b"b:2".to_vec());
        let s = scanned(&[b"a:1", b"b:1", b"b:2", b"c:1"], restored);
        let i = s.selected_key().unwrap();
        assert_eq!(s.keys.name(i).unwrap(), b"b:2");
    }
}
