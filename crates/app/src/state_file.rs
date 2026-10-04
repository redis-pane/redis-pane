//! `$XDG_STATE_HOME/redis-pane/state.json`: session restore's file I/O
//! (M4 task 7, `docs/plans/m4-session-restore.md`, ADR-0003).
//!
//! The format, per-target keying and snapshot/apply live in
//! `redis_pane_core::state::session`; this module only reads and writes. It
//! never touches `config.json` — the app only ever reads that.
//!
//! Writes are atomic (temp file in the same directory, then rename) and happen
//! on a dedicated thread, so neither the render loop nor the tokio workers
//! ever wait on the disk. [`Persister`] decides *when*: a change starts (or
//! restarts) a debounce, and quitting flushes whatever is pending.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use redis_pane_core::clock::Clock;
use redis_pane_core::state::{SessionFile, SessionFileError, SessionState};

/// How long a change waits for a quieter moment before it is written. A
/// filter typed character by character is one write, not one per keystroke.
pub const DEBOUNCE: Duration = Duration::from_millis(1_000);

/// `$XDG_STATE_HOME/redis-pane/state.json`, falling back to
/// `~/.local/state/redis-pane/state.json` (the XDG default).
pub fn default_path() -> Option<PathBuf> {
    path_from(
        std::env::var("XDG_STATE_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

fn path_from(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(xdg) = xdg.filter(|x| !x.is_empty()) {
        return Some(PathBuf::from(xdg).join("redis-pane/state.json"));
    }
    home.filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join(".local/state/redis-pane/state.json"))
}

/// What loading found. `notice` is set when a file existed but was unusable:
/// the session proceeds without restored state, and the reader is told once.
#[derive(Debug, Default)]
pub struct Loaded {
    pub session: Option<SessionState>,
    pub notice: Option<String>,
}

/// Read the remembered session for `target`. A missing file is normal; an
/// unreadable, corrupt or old one is ignored with a notice, never an error.
pub fn load(path: &Path, target: &str) -> Loaded {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Loaded::default(),
        Err(e) => {
            return Loaded {
                session: None,
                notice: Some(format!("{}: {e}", path.display())),
            };
        }
    };
    match SessionFile::parse(&text) {
        Ok(file) => Loaded {
            session: file.get(target).cloned(),
            notice: None,
        },
        Err(e) => Loaded {
            session: None,
            notice: Some(format!("{}: {}", path.display(), describe(&e))),
        },
    }
}

fn describe(e: &SessionFileError) -> String {
    format!("{e}; ignoring it and starting without a restored session")
}

/// Merge one target's session into the file and write it atomically.
///
/// The file is re-read first so other targets' sessions (possibly written by
/// another terminal) survive; a file that cannot be read is replaced.
pub fn save(path: &Path, target: &str, state: SessionState, now_ms: u64) -> std::io::Result<()> {
    let mut file = std::fs::read_to_string(path)
        .ok()
        .and_then(|t| SessionFile::parse(&t).ok())
        .unwrap_or_default();
    file.put(target, state, now_ms);
    let temp = write_temp(path, file.to_json().as_bytes())?;
    commit(&temp, path)
}

fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".tmp.{}", std::process::id()));
    path.with_file_name(name)
}

/// First half of an atomic write: the complete new content, on disk, next to
/// the destination. The destination is untouched until [`commit`].
fn write_temp(path: &Path, bytes: &[u8]) -> std::io::Result<PathBuf> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = temp_path(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // Filters and key names are private to the reader.
        options.mode(0o600);
    }
    let mut f = options.open(&temp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(temp)
}

/// Second half: the rename that makes the new content visible all at once.
fn commit(temp: &Path, path: &Path) -> std::io::Result<()> {
    std::fs::rename(temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(temp);
    })
}

/// Where and under which key this session is saved.
#[derive(Debug, Clone)]
pub struct Store {
    pub path: PathBuf,
    /// The display target (`host:port/db`), already free of credentials.
    pub target: String,
}

/// The write thread: receives snapshots, coalesces to the newest, writes.
struct Writer {
    tx: Sender<SessionState>,
    handle: JoinHandle<Option<String>>,
}

impl Writer {
    /// `on_error` is told about each failed write as it happens (the shell
    /// turns it into a notification); the thread's return value is whether
    /// its *last* write failed, for the quit path to report.
    fn spawn(
        store: Store,
        clock: Arc<dyn Clock>,
        on_error: impl Fn(String) + Send + 'static,
    ) -> Self {
        let (tx, rx): (_, Receiver<SessionState>) = channel();
        let handle = std::thread::spawn(move || {
            let mut last_error = None;
            let mut reported = false;
            while let Ok(mut state) = rx.recv() {
                while let Ok(newer) = rx.try_recv() {
                    state = newer;
                }
                match save(&store.path, &store.target, state, clock.now_epoch_ms()) {
                    Ok(()) => {
                        last_error = None;
                        reported = false;
                    }
                    Err(e) => {
                        let text = format!("{}: {e}", store.path.display());
                        // Once per outage: a full disk would otherwise raise
                        // the same error on every debounced change.
                        if !reported {
                            on_error(text.clone());
                            reported = true;
                        }
                        last_error = Some(text);
                    }
                }
            }
            last_error
        });
        Writer { tx, handle }
    }
}

/// Decides when a changed session is written.
pub struct Persister {
    /// The last session seen, so only a *change* arms the debounce.
    seen: SessionState,
    pending: Option<(SessionState, Instant)>,
    writer: Writer,
}

impl Persister {
    pub fn new(
        store: Store,
        initial: SessionState,
        clock: Arc<dyn Clock>,
        on_error: impl Fn(String) + Send + 'static,
    ) -> Self {
        Persister {
            seen: initial,
            pending: None,
            writer: Writer::spawn(store, clock, on_error),
        }
    }

    /// Note the current session. A change (re)starts the debounce.
    pub fn observe(&mut self, now: Instant, snapshot: SessionState) {
        if snapshot != self.seen {
            self.pending = Some((snapshot.clone(), now + DEBOUNCE));
            self.seen = snapshot;
        }
    }

    /// When the loop should next wake to write, if a write is pending.
    pub fn deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|(_, at)| *at)
    }

    /// Hand the pending session to the writer if its debounce has elapsed.
    /// Returns whether it did.
    pub fn flush_if_due(&mut self, now: Instant) -> bool {
        match &self.pending {
            Some((_, at)) if *at <= now => {
                if let Some((state, _)) = self.pending.take() {
                    let _ = self.writer.tx.send(state);
                }
                true
            }
            _ => false,
        }
    }

    /// Quit: write the final session (whether or not a debounce was pending),
    /// wait for the writer, and report whether the last write failed.
    pub fn finish(self, final_state: SessionState) -> Option<String> {
        let Persister {
            seen,
            pending,
            writer,
        } = self;
        // Only write if something differs from what is already on disk or in
        // flight: a session that changed nothing leaves the file alone.
        if pending.is_some() || final_state != seen {
            let _ = writer.tx.send(final_state);
        }
        drop(writer.tx);
        writer.handle.join().ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis_pane_core::clock::FixedClock;
    use redis_pane_core::state::{FilterMode, SortBy};
    use std::sync::Mutex;

    fn session(filter: &str) -> SessionState {
        SessionState {
            split_adjust: 3,
            tree_mode: true,
            sort: SortBy::Name,
            filter: filter.into(),
            filter_mode: FilterMode::Glob,
            selected_key: Some(vec![0xff, b'k']),
        }
    }

    fn tempdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rp-state-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn persister(path: &Path, initial: SessionState) -> Persister {
        Persister::new(
            Store {
                path: path.to_path_buf(),
                target: "t:6379/0".into(),
            },
            initial,
            Arc::new(FixedClock(42)),
            |_| {},
        )
    }

    #[test]
    fn path_follows_xdg_state_home_then_the_xdg_default() {
        assert_eq!(
            path_from(Some("/x"), Some("/h")),
            Some(PathBuf::from("/x/redis-pane/state.json"))
        );
        assert_eq!(
            path_from(Some(""), Some("/h")),
            Some(PathBuf::from("/h/.local/state/redis-pane/state.json"))
        );
        assert_eq!(path_from(None, None), None);
    }

    #[test]
    fn a_saved_session_loads_for_its_target_only() {
        let dir = tempdir("roundtrip");
        let path = dir.join("sub/state.json");
        save(&path, "a:6379/0", session("x*"), 1).unwrap();
        save(&path, "b:6379/0", session("y*"), 2).unwrap();
        assert_eq!(load(&path, "a:6379/0").session, Some(session("x*")));
        assert_eq!(load(&path, "b:6379/0").session, Some(session("y*")));
        assert!(load(&path, "c:6379/0").session.is_none());
        assert!(load(&path, "c:6379/0").notice.is_none());
    }

    #[test]
    fn a_missing_file_is_silent() {
        let dir = tempdir("missing");
        let l = load(&dir.join("nope.json"), "t");
        assert!(l.session.is_none() && l.notice.is_none());
    }

    #[test]
    fn a_corrupt_file_is_ignored_with_a_notice() {
        let dir = tempdir("corrupt");
        let path = dir.join("state.json");
        std::fs::write(&path, "{ definitely not json").unwrap();
        let l = load(&path, "t");
        assert!(l.session.is_none());
        let notice = l.notice.expect("the reader must be told");
        assert!(notice.contains("state.json"), "{notice}");
    }

    #[test]
    fn an_unrecognised_version_is_ignored_with_a_notice() {
        let dir = tempdir("version");
        let path = dir.join("state.json");
        std::fs::write(&path, r#"{"version": 7, "targets": {}}"#).unwrap();
        let l = load(&path, "t");
        assert!(l.session.is_none());
        assert!(l.notice.unwrap().contains("version 7"));
    }

    #[test]
    fn saving_over_a_corrupt_file_replaces_it() {
        let dir = tempdir("replace");
        let path = dir.join("state.json");
        std::fs::write(&path, "garbage").unwrap();
        save(&path, "t", session("z"), 1).unwrap();
        assert_eq!(load(&path, "t").session, Some(session("z")));
    }

    #[test]
    fn an_interrupted_write_leaves_the_original_untouched() {
        let dir = tempdir("atomic");
        let path = dir.join("state.json");
        save(&path, "t", session("old"), 1).unwrap();
        let before = std::fs::read(&path).unwrap();
        // The process dies after the temp file is written, before the rename.
        let temp = write_temp(&path, b"half-finished new content").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(load(&path, "t").session, Some(session("old")));
        commit(&temp, &path).unwrap();
        assert_ne!(std::fs::read(&path).unwrap(), before);
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir("mode");
        let path = dir.join("state.json");
        save(&path, "t", session("a"), 1).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn no_change_means_no_write() {
        let dir = tempdir("nochange");
        let path = dir.join("state.json");
        let mut p = persister(&path, session("a"));
        let now = Instant::now();
        p.observe(now, session("a"));
        assert!(p.deadline().is_none());
        assert!(p.finish(session("a")).is_none());
        assert!(!path.exists(), "an unchanged session writes nothing");
    }

    #[test]
    fn rapid_changes_coalesce_into_one_write_after_the_last() {
        let dir = tempdir("debounce");
        let path = dir.join("state.json");
        let mut p = persister(&path, session(""));
        let t0 = Instant::now();
        for (i, f) in ["a", "ab", "abc"].iter().enumerate() {
            p.observe(t0 + Duration::from_millis(100 * i as u64), session(f));
        }
        // Each change pushed the deadline out from the latest one.
        let deadline = p.deadline().unwrap();
        assert_eq!(deadline, t0 + Duration::from_millis(200) + DEBOUNCE);
        assert!(!p.flush_if_due(t0 + Duration::from_millis(900)));
        assert!(!path.exists());
        assert!(p.flush_if_due(deadline));
        assert!(p.deadline().is_none());
        assert!(p.finish(session("abc")).is_none());
        assert_eq!(load(&path, "t:6379/0").session, Some(session("abc")));
    }

    #[test]
    fn quit_flushes_a_pending_write_instead_of_losing_it() {
        let dir = tempdir("quit");
        let path = dir.join("state.json");
        let mut p = persister(&path, session(""));
        p.observe(Instant::now(), session("typed just now"));
        assert!(p.deadline().is_some(), "still inside the debounce");
        assert!(p.finish(session("typed just now")).is_none());
        assert_eq!(
            load(&path, "t:6379/0").session,
            Some(session("typed just now"))
        );
    }

    #[test]
    fn a_failed_write_is_reported_once_and_on_quit() {
        let dir = tempdir("fail");
        // The "file" is a directory, so the rename must fail.
        let path = dir.join("state.json");
        std::fs::create_dir_all(path.join("occupied")).unwrap();
        let errors = Arc::new(Mutex::new(Vec::new()));
        let sink = errors.clone();
        let mut p = Persister::new(
            Store {
                path: path.clone(),
                target: "t".into(),
            },
            session(""),
            Arc::new(FixedClock(1)),
            move |e| sink.lock().unwrap().push(e),
        );
        let t0 = Instant::now();
        p.observe(t0, session("a"));
        p.flush_if_due(t0 + DEBOUNCE);
        p.observe(t0 + DEBOUNCE, session("b"));
        let last = p.finish(session("b"));
        assert!(
            last.is_some(),
            "the quit path learns the final write failed"
        );
        assert_eq!(
            errors.lock().unwrap().len(),
            1,
            "one notification per outage"
        );
    }
}
