//! The Monitor view's state (`g m`, R6.1, M3 phase B, `docs/plans/m3-monitor.md`):
//! the bounded tail buffer, the feed connection's own status, and
//! pause/filter/following.
//!
//! `FeedStatus` lives here, not on `State` directly, because
//! `docs/plans/m3-feed-connection.md` built it generic over "whichever
//! feature opens a feed" and said Monitor's own state should hold it once
//! there was a real feature to embed it in — this is that move.

use crate::command::FeedToken;
use std::collections::VecDeque;

/// Every scanned/streamed line is retained up to this cap — ADR-0010's
/// discipline ("every key scanned so far is retained up to a documented
/// cap... enforced in exactly one place") applied to a stream instead of a
/// scan (`docs/plans/m3-monitor.md` decision 4).
pub const MONITOR_CAP: usize = 5_000;

/// A single line's raw text is truncated to this many bytes on arrival — a
/// single `SET` with a 10MB value would otherwise make one line bigger than
/// the whole buffer is meant to be. Decision 4: worst case is therefore
/// `MONITOR_CAP * MONITOR_LINE_MAX` ≈ 20MB, not unbounded.
pub const MONITOR_LINE_MAX: usize = 4 * 1024;

/// Where a push/poll feed connection stands (`docs/plans/m3-feed-connection.md`).
/// Each feature that opens one — Monitor now, Pub/Sub later — embeds this
/// rather than the core holding a `Vec<FeedHandle>`: the core cannot hold a
/// `fred` handle at all (ADR-0011).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FeedStatus {
    #[default]
    Idle,
    Connecting,
    Open,
    Closed {
        reason: Option<String>,
    },
}

/// One line, as received from the shell's `MONITOR` read loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorLine {
    /// Receipt time (the shell's injected `Clock`, ADR-0011) — local buffer
    /// ordering is a pure function of injected state, never of the server's
    /// own clock (which the raw text also carries, and which the column
    /// display prefers — see [`MonitorLine::columns`]).
    pub at_ms: u64,
    /// The server's own `MONITOR` line, already truncated to
    /// [`MONITOR_LINE_MAX`] if it ran over.
    pub raw: String,
    pub truncated: bool,
}

/// The server-time columns parsed out of a [`MonitorLine::raw`] (decision 8):
/// `<ts> [<db> <client>] "<command>" "<arg>" ...`, fred's own `Display` for a
/// parsed `MONITOR` line, matching `redis-cli`'s own formatting.
///
/// Parsing rather than storing these fields separately from the start keeps
/// phase A's `Msg::MonitorLine` (`crates/app/src/redis/feed.rs`) simple — it
/// only ever hands the core the one string `fred` already built — and proves
/// the parser against exactly what a real server sends (the integration
/// test). A line that doesn't parse (any of the fixed shape's markers
/// missing) reports `time: None` and the whole raw text as `command`, rather
/// than failing — the tail keeps showing *something* for a line it couldn't
/// fully parse, never nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorColumns<'a> {
    /// `HH:MM:SS.mmm`, UTC, from the server's own timestamp — preferred over
    /// receipt time for display (decision 8), since it is what the reader is
    /// actually asking "when did this run" about.
    pub time: Option<String>,
    pub db: &'a str,
    pub client: &'a str,
    /// The quoted command and its arguments, exactly as the server sent them
    /// — `"SET" "key" "value"` — unescaped further here; [`crate::state::value::cell_text`]
    /// is what the render layer runs this through for the Viewer's own byte
    /// escaping (decision 8: "shown with the Viewer's byte escaping, never
    /// raw") and what `c` copies.
    pub command: &'a str,
}

impl MonitorLine {
    /// Parse `raw` into its columns. Pure string slicing — cheap enough to
    /// call at render time rather than cached on the line.
    pub fn columns(&self) -> MonitorColumns<'_> {
        parse_monitor_line(&self.raw)
    }
}

fn parse_monitor_line(raw: &str) -> MonitorColumns<'_> {
    let fallback = MonitorColumns {
        time: None,
        db: "",
        client: "",
        command: raw,
    };
    let Some((ts_str, rest)) = raw.split_once(' ') else {
        return fallback;
    };
    let Some(rest) = rest.strip_prefix('[') else {
        return fallback;
    };
    let Some((bracket, command)) = rest.split_once(']') else {
        return fallback;
    };
    let Some((db, client)) = bracket.split_once(' ') else {
        return fallback;
    };
    let time = ts_str.parse::<f64>().ok().map(format_utc_time);
    MonitorColumns {
        time,
        db,
        client,
        // The one space fred's `Display` always writes between `]` and the
        // first quoted argument.
        command: command.strip_prefix(' ').unwrap_or(command),
    }
}

/// Epoch seconds (fractional, `MONITOR`'s own timestamp) to `HH:MM:SS.mmm`
/// UTC. Only the time of day is needed here — never a date — so this is
/// plain arithmetic on the seconds-since-midnight remainder, not a calendar
/// computation: no date crate belongs in the core for one column (ADR-0011's
/// spirit, kept literal rather than merely followed by convention).
fn format_utc_time(epoch_secs: f64) -> String {
    let total_ms = (epoch_secs * 1000.0).round() as i64;
    let total_ms = total_ms.rem_euclid(86_400_000);
    let ms = total_ms % 1000;
    let total_secs = total_ms / 1000;
    let s = total_secs % 60;
    let m = (total_secs / 60) % 60;
    let h = (total_secs / 3600) % 24;
    format!("{h:02}:{m:02}:{s:02}.{ms:03}")
}

/// The Monitor view's whole state (`View::Monitor`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MonitorState {
    lines: VecDeque<MonitorLine>,
    /// Where the feed connection stands (`docs/plans/m3-feed-connection.md`).
    pub status: FeedStatus,
    /// Identifies the most recently issued `Command::OpenFeed` for this view
    /// — mirrors `State::read_token`'s discipline: minted only by
    /// [`crate::update::update`], so a shell reply naming an old feed cannot
    /// be mistaken for one about the feed currently open.
    pub feed_token: FeedToken,
    pub paused: bool,
    /// Lines that arrived while paused — counted, never buffered (decision
    /// 5: "a paused Monitor is not a queue, it is a closed valve"). Reset to
    /// `0` on resume, so each pause reports its own count rather than an
    /// ever-growing session total.
    pub dropped_while_paused: u64,
    /// The `/`-typed pattern narrowing what's *displayed* — never what's
    /// buffered (decision 6). Matched with [`crate::state::view::matches`],
    /// the same predicate the keys pane's own filter uses.
    pub filter: String,
    selected: usize,
    /// Whether the view is following new lines as they arrive (decision 7).
    /// Any movement off the last line turns this off without pausing
    /// consumption; jumping to the bottom (`End`/`Action::Bottom`) turns it
    /// back on.
    pub following: bool,
}

impl MonitorState {
    pub fn lines(&self) -> &VecDeque<MonitorLine> {
        &self.lines
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_line(&self) -> Option<&MonitorLine> {
        self.lines.get(self.selected)
    }

    /// Start a fresh view: every `g m` opens a fresh connection, nothing is
    /// resumed (decision 2). Resetting the whole buffer — not just the feed
    /// status — is what keeps a feed that silently died in a past session
    /// from ever being mistaken for the one currently open, and is what
    /// decision 2's "nothing is resumed" means for the *tail*, not only the
    /// socket.
    pub fn reset(&mut self) {
        let feed_token = self.feed_token;
        *self = MonitorState {
            feed_token,
            following: true,
            ..MonitorState::default()
        };
    }

    /// The one function that appends to `lines` (CLAUDE.md: "the cap is
    /// enforced in exactly one place"; ADR-0010's naming discipline extended
    /// to a stream, decision 4). Truncates to [`MONITOR_LINE_MAX`] and evicts
    /// from the front past [`MONITOR_CAP`].
    ///
    /// Paused: the line is not appended at all — counted in
    /// `dropped_while_paused` and nothing else (decision 5). This is the
    /// difference between pause stopping *consumption* and merely *hiding*
    /// the pane: `lines` does not grow while paused, regardless of how many
    /// lines the socket delivers underneath it.
    pub fn push_monitor_line(&mut self, at_ms: u64, mut raw: String) {
        if self.paused {
            self.dropped_while_paused += 1;
            return;
        }
        let truncated = raw.len() > MONITOR_LINE_MAX;
        if truncated {
            // Cut on a char boundary: `raw` came through as a `String`
            // (already lossily decoded, `MonitorCommand`'s own `Display`),
            // and truncating mid-codepoint would panic.
            let mut cut = MONITOR_LINE_MAX;
            while cut > 0 && !raw.is_char_boundary(cut) {
                cut -= 1;
            }
            raw.truncate(cut);
        }
        self.lines.push_back(MonitorLine {
            at_ms,
            raw,
            truncated,
        });
        if self.lines.len() > MONITOR_CAP {
            self.lines.pop_front();
            // A position, not a stable id: eviction shifts every later line
            // down by one, and the line at `selected` shifts with it. `0`
            // clamps to the new oldest line (decision 7); it does not move
            // further, since the line it names is still there.
            self.selected = self.selected.saturating_sub(1);
        }
        if self.following {
            self.selected = self.lines.len().saturating_sub(1);
        }
    }

    /// Movement within the tail. Any move away from the last line stops
    /// following (decision 7) — resuming is `to_bottom`'s job alone.
    pub fn move_selection(&mut self, delta: isize) {
        if self.lines.is_empty() {
            return;
        }
        self.following = false;
        let max = (self.lines.len() - 1) as isize;
        let new = (self.selected as isize + delta).clamp(0, max);
        self.selected = new as usize;
    }

    pub fn to_top(&mut self) {
        self.following = false;
        self.selected = 0;
    }

    /// `End`/`Action::Bottom`: jump to the newest line and resume following
    /// (decision 7).
    pub fn to_bottom(&mut self) {
        self.following = true;
        self.selected = self.lines.len().saturating_sub(1);
    }

    /// `p`: flip pause. Resuming clears the drop count — a fresh count for
    /// the next pause, not a running session total (decision 5's "paused ·
    /// N skipped" is a report on *this* pause).
    pub fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            self.dropped_while_paused = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cap_is_never_exceeded_over_a_long_synthetic_run() {
        let mut m = MonitorState::default();
        for i in 0..(MONITOR_CAP * 3) {
            m.push_monitor_line(i as u64, format!("line {i}"));
            assert!(m.len() <= MONITOR_CAP, "grew past the cap at {i}");
        }
        assert_eq!(m.len(), MONITOR_CAP);
        // The oldest surviving line is the most recent `MONITOR_CAP` of them.
        assert_eq!(
            m.lines().front().unwrap().raw,
            format!("line {}", MONITOR_CAP * 3 - MONITOR_CAP)
        );
    }

    #[test]
    fn a_line_over_the_byte_cap_is_truncated_and_marked() {
        let mut m = MonitorState::default();
        let huge = "x".repeat(MONITOR_LINE_MAX * 3);
        m.push_monitor_line(1, huge);
        let line = m.lines().back().unwrap();
        assert_eq!(line.raw.len(), MONITOR_LINE_MAX);
        assert!(line.truncated);
    }

    #[test]
    fn a_short_line_is_not_marked_truncated() {
        let mut m = MonitorState::default();
        m.push_monitor_line(1, "short".to_string());
        assert!(!m.lines().back().unwrap().truncated);
    }

    #[test]
    fn pausing_counts_lines_rather_than_buffering_them() {
        let mut m = MonitorState::default();
        m.push_monitor_line(1, "one".into());
        m.toggle_pause();
        assert!(m.paused);
        for i in 0..10 {
            m.push_monitor_line(i, format!("dropped {i}"));
        }
        assert_eq!(m.len(), 1, "the buffer never grew while paused");
        assert_eq!(m.dropped_while_paused, 10);
    }

    #[test]
    fn resuming_does_not_backfill_what_was_dropped() {
        let mut m = MonitorState::default();
        m.toggle_pause();
        m.push_monitor_line(1, "dropped".into());
        assert_eq!(m.dropped_while_paused, 1);
        m.toggle_pause(); // resume
        assert!(!m.paused);
        assert_eq!(m.len(), 0, "nothing was ever buffered to backfill");
        assert_eq!(m.dropped_while_paused, 0, "the count resets on resume");
        m.push_monitor_line(2, "live again".into());
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn following_tracks_new_lines_until_moved_off_the_end() {
        let mut m = MonitorState {
            following: true,
            ..MonitorState::default()
        };
        for i in 0..5 {
            m.push_monitor_line(i, format!("l{i}"));
        }
        assert_eq!(m.selected(), 4, "still following");
        m.move_selection(-1);
        assert!(!m.following);
        assert_eq!(m.selected(), 3);
        m.push_monitor_line(5, "l5".into());
        assert_eq!(
            m.selected(),
            3,
            "not following anymore, so a new line must not move the selection"
        );
        m.to_bottom();
        assert!(m.following);
        assert_eq!(m.selected(), 5);
    }

    #[test]
    fn eviction_clamps_a_selection_on_the_oldest_line_rather_than_panicking() {
        let mut m = MonitorState::default();
        for i in 0..3 {
            m.push_monitor_line(i, format!("l{i}"));
        }
        m.to_top();
        assert_eq!(m.selected(), 0);
        // Push past a tiny effective cap by pretending MONITOR_CAP is small:
        // exercised directly against the real cap in the long-run test above;
        // here we only need eviction with `selected == 0` to stay at `0`
        // rather than underflow.
        for i in 0..MONITOR_CAP {
            m.push_monitor_line(100 + i as u64, format!("fill {i}"));
        }
        assert_eq!(m.selected(), 0);
    }

    #[test]
    fn reset_clears_everything_but_keeps_moving_the_feed_token_forward_untouched() {
        let mut m = MonitorState::default();
        m.push_monitor_line(1, "l".into());
        m.paused = true;
        m.filter = "SET".into();
        m.following = false;
        let token = m.feed_token;
        m.reset();
        assert!(m.is_empty());
        assert!(!m.paused);
        assert!(m.filter.is_empty());
        assert!(m.following, "a fresh view follows the tail by default");
        assert_eq!(m.feed_token, token, "reset does not mint a token itself");
    }

    #[test]
    fn columns_parse_a_real_monitor_line() {
        let line = MonitorLine {
            at_ms: 0,
            raw: r#"1339518083.107412 [0 127.0.0.1:60866] "set" "key" "value""#.to_string(),
            truncated: false,
        };
        let cols = line.columns();
        assert_eq!(cols.time.as_deref(), Some("16:21:23.107"));
        assert_eq!(cols.db, "0");
        assert_eq!(cols.client, "127.0.0.1:60866");
        assert_eq!(cols.command, r#""set" "key" "value""#);
    }

    #[test]
    fn an_unparseable_line_falls_back_to_showing_the_whole_raw_text() {
        let line = MonitorLine {
            at_ms: 0,
            raw: "not a monitor line at all".to_string(),
            truncated: false,
        };
        let cols = line.columns();
        assert_eq!(cols.time, None);
        assert_eq!(cols.command, "not a monitor line at all");
    }
}
