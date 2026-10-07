//! Contextual help (`?` / `F1`): one model of "what the keys do here", read
//! by both the help overlay and the hint bar (PLAN `docs/plans/m3-contextual-help.md`).
//!
//! This module only derives *what to show*; drawing it (the overlay, the
//! hint bar's truncation to width) is `render`'s job, not this one. The
//! precedence used to tell contexts apart is `update::mode_beneath_help`'s —
//! reused, not re-derived, so the two can never disagree about which mode a
//! keypress would land in. Deliberately not `update::mode` itself: help is
//! drawn *over* a context, never instead of one (closing it must reveal
//! exactly the mode that was showing before it opened), and `mode` itself
//! would just answer `Mode::Help` once help is open, which says nothing
//! about what help is covering.
//!
//! Every domain rule here has a citation back to the `update` function that
//! actually enforces it: this module must never invent a refusal dispatch
//! would not actually apply (`docs/plans/m3-contextual-help.md`, D3).

use std::borrow::Cow;

use crate::State;
use crate::keymap::{Action, key_label};
use crate::render::layout::Pane;
use crate::state::value::Value;
use crate::state::{EditTarget, FieldPart, Link, OpenKey, ReadOnlyReason};
use crate::update::{Mode, mode_beneath_help};

/// The focused context help and the hint bar both describe. Derived from
/// [`State`] alone, in the same precedence [`update::mode`] uses — never
/// stored, so it cannot disagree with the mode a keypress actually lands in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpContext {
    /// The keys pane, focused. `tree` is which view is on screen (D2,
    /// ADR-0019's `t` toggle); `filtered` is whether `/` has narrowed it —
    /// both change wording (a folded row's `t` unfolds; an applied filter
    /// gets a "clear filter" mention) but not which actions apply.
    Keys { tree: bool, filtered: bool },
    /// `/` is capturing a filter pattern (`update::filter_key`). Every
    /// ordinary character is text here, not a command — see `update::mode`'s
    /// own comment for why this outranks Normal mode.
    Filter,
    /// `R` is capturing a new key name (M2 task 11): the `Filter`-shaped
    /// sibling for `State::rename`.
    Rename,
    /// The Viewer, focused, with `open` describing what's in it — `None`
    /// while nothing has been read yet (freshly opened, or read pending).
    Value(Option<ValueContext>),
    /// The inline editor has a buffer open (`update::mode`: outranks Filter,
    /// since a key open for editing outranks a filter left running in the
    /// background).
    Editor(EditorContext),
    /// A mutation is staged, waiting on `y`/`Esc` (`update::confirm_key`).
    /// Ranked first by `update::mode` — above Editor and Filter — since a
    /// staged mutation is the only thing that can act on a keypress until
    /// it resolves.
    Confirm,
    /// A `g`-chord's prefix has been pressed and is waiting on its second key
    /// (M3, `docs/plans/m3-slowlog.md`) — checked first among the
    /// `Mode::Normal` contexts, since a pending chord is the only thing that
    /// can act on the very next keypress until it resolves, the same reason
    /// `Confirm` outranks everything beneath it.
    ChordPending,
    /// The Slowlog view (`g s`, R6.4) — a full screen, not the two-pane
    /// browser, so it gets one context rather than being folded into
    /// [`HelpContext::Keys`]/[`HelpContext::Value`].
    Slowlog,
    /// The Monitor view (`g m`, R6.1, M3 phase B, `docs/plans/m3-monitor.md`)
    /// — the same "a full screen, not the two-pane browser" reasoning as
    /// [`HelpContext::Slowlog`], one context of its own.
    Monitor,
    /// `a` is capturing a subscription's channel/pattern text
    /// (`update::pubsub::pubsub_add_key`) — the `Filter`-shaped sibling
    /// context for `PubSubState::input`. Every ordinary character is text
    /// here too, same reason `Filter` outranks Normal mode.
    PubSubAdding,
    /// The Pub/Sub view (`g p`, R6.2, M3 task 5, `docs/plans/m3-pubsub.md`)
    /// — the same "a full screen, not the two-pane browser" reasoning as
    /// [`HelpContext::Slowlog`]/[`HelpContext::Monitor`]. `focus` picks the
    /// strip-scoped or tail-scoped rows (decision 1).
    PubSub { focus: crate::state::PubSubFocus },
    /// The Dashboard view (`g d`, R6.3, M3 task 6,
    /// `docs/plans/m3-dashboard.md`) — the same "a full screen, not the
    /// two-pane browser" reasoning as [`HelpContext::Slowlog`]/
    /// [`HelpContext::Monitor`]/[`HelpContext::PubSub`], one context of its
    /// own. Not parameterised by the focused tile: every tile's help rows
    /// are the same four actions (move, expand, refetch, copy) regardless
    /// of which tile focus happens to be on.
    Dashboard,
    /// The Dashboard on a Cluster, before any node is opened (M5 task 7,
    /// `docs/plans/m5-dashboard.md`): the node table, whose rows move and
    /// open, not the tile grid's. A node view is [`HelpContext::Dashboard`].
    DashboardCluster,
    /// The Dashboard's raw-`INFO` overlay, open over the grid (decision 6's
    /// `Enter`). A separate context, not [`HelpContext::Dashboard`] with a
    /// flag: the overlay's own keys (scroll, copy, `Esc` to close) are a
    /// different, smaller set than the grid's (move focus, expand, refetch,
    /// copy) — the same reasoning that gives `HelpContext::Confirm` and
    /// `HelpContext::ChordPending` their own variants rather than folding
    /// into whatever context they are drawn over.
    DashboardOverlay,
}

/// The Viewer's per-type contexts (PLAN's decision 2): one for each of the
/// eight [`Value`] variants, crossed with whether the value cursor is active
/// (`OpenKey::cursor_active`, set by `Enter`/`Action::EnterValueCursor`) —
/// the row-level actions (`e`/`a`/`d`) all gate on it for every collection
/// type (`update::viewer::delete_value_row`, `update::editor::open_editor`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueContext {
    /// `cursor` is carried even though nothing here reads a row through it —
    /// `Enter`/`Action::EnterValueCursor` activates it for any type with a
    /// value (`enter_value_cursor`'s gate is "has a value", not "is a
    /// collection"), and `Action::pane_is_on_screen`'s movement group reads
    /// it regardless of type, so two states that render different motion
    /// labels must not collapse onto the same context.
    Str {
        cursor: bool,
    },
    Hash {
        cursor: bool,
    },
    List {
        cursor: bool,
    },
    Set {
        cursor: bool,
    },
    ZSet {
        cursor: bool,
    },
    /// A Stream has no row-level actions yet (no edit, no add, no delete),
    /// but it does have rows to read through once `Enter` puts a cursor on
    /// one.
    Stream {
        cursor: bool,
    },
    Json {
        cursor: bool,
    },
    /// Never editable in place (`EditBuffer::from_value`'s refusal) and
    /// never addable/removable (`begin_add_entry`/`delete_value_row`'s
    /// refusals) — only Copy and the TTL editor apply.
    Binary {
        cursor: bool,
    },
}

impl ValueContext {
    fn cursor(&self) -> bool {
        match *self {
            ValueContext::Str { cursor }
            | ValueContext::Hash { cursor }
            | ValueContext::List { cursor }
            | ValueContext::Set { cursor }
            | ValueContext::ZSet { cursor }
            | ValueContext::Stream { cursor }
            | ValueContext::Json { cursor }
            | ValueContext::Binary { cursor } => cursor,
        }
    }
}

/// The inline editor's contexts, one per shape [`EditTarget`] takes
/// (`update::editor`'s doc comment: "opening a buffer on a String/JSON value
/// or a Hash field, the two-part add-field form, staging an edit").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorContext {
    /// [`EditTarget::Value`]: a String or JSON-classified String's whole
    /// body. The only editor target staged through the rebindable
    /// `Action::EditorStage`, not literal `Enter` — a multi-line body needs
    /// `Enter` to insert a newline (ADR-0014).
    Value,
    /// [`EditTarget::HashField`], [`EditTarget::NewSetMember`] (mid-capture,
    /// i.e. its `active_part()` is already `None`) and
    /// [`EditTarget::ListElement`]: a single-buffer edit with no `FIELD`/
    /// `VALUE` split, staged with literal `Enter` (the 2026-09-22 amendment
    /// to ADR-0014: `Enter` stages everything but a plain String/JSON body).
    Field,
    /// [`EditTarget::NewHashField`]/[`EditTarget::NewZSetMember`] on
    /// [`FieldPart::Name`]: capturing the field/member name before the value
    /// half opens. `zset` picks the noun ("member" vs "field") for the
    /// status line's duplicate warning.
    AddFormName { zset: bool },
    /// The same two targets on [`FieldPart::Value`]: capturing the value/
    /// score, with `↑` returning to the name part once the cursor has
    /// nowhere left to go.
    AddFormValue { zset: bool },
    /// [`EditTarget::NewListElement`]: `Tab` flips head/tail — the only
    /// place on the whole bar `Tab` means anything (`hint_bar`'s comment).
    ListAdd,
    /// [`EditTarget::ZSetScore`]: an existing member's score, staged through
    /// `Action::EditorStage` like a plain Value edit (no literal `Enter`),
    /// but with the live invalid-score indicator the add form's score part
    /// also shows.
    ZSetScore,
    /// [`EditTarget::Ttl`]: no undo (typing nothing is itself a valid
    /// write), and the bar never shows a live invalid/valid indicator here,
    /// unlike `ZSetScore`, above (D13).
    Ttl,
}

/// One row: an effective binding, what it does here, and why it's refused,
/// if it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelpRow {
    /// The effective binding(s), spelled the way [`key_label`] does — merged
    /// when more than one key invokes the same action (`? F1` for Help),
    /// or the literal key for an editor key that isn't a rebindable
    /// [`Action`] at all (`Enter`, `Tab` — `hint_bar`'s own comment on why).
    pub keys: String,
    /// Names the target, never describes it (PLAN's decision 6): "delete
    /// field", not "delete". No description column.
    pub label: Cow<'static, str>,
    /// `None` when the row does exactly what it says. `Some` when it's
    /// refused right now — shown dimmed with the reason, never hidden
    /// (PLAN's decision 3).
    pub refused: Option<Refusal>,
}

impl HelpRow {
    fn new(keys: impl Into<String>, label: impl Into<Cow<'static, str>>) -> Self {
        HelpRow {
            keys: keys.into(),
            label: label.into(),
            refused: None,
        }
    }

    fn refused(mut self, refusal: Refusal) -> Self {
        self.refused = Some(refusal);
        self
    }
}

/// Why a row is refused right now. The core's dispatch only ever refuses on
/// `y` (`update::confirm_key`'s read-only check) — Read-only Mode does not
/// block opening the editor or staging a preview (`update::mode`'s comment:
/// "Read-only Mode is decided at confirm, not at staging"). Every row that
/// *starts* down that path — stages a mutation directly (`d`), or opens the
/// editor that leads to one (`e`, `a`, and `t` meaning edit ttl) — is still
/// dimmed with the same reason, so the reader learns the outcome before
/// typing a whole value the confirm dialog will refuse to run, not after.
/// `y` itself reads differently — see `preview_only` below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    pub reason: ReadOnlyReason,
    /// `true` for a row that still runs and stages a preview despite the
    /// refusal (`update::mode`'s comment: "a staged mutation is decided at
    /// confirm, not at the keypress that staged it") — the reason gets a
    /// `· preview only` suffix so the dimming does not imply the keypress
    /// does nothing. `false` for the confirm dialog's `y` row, which really
    /// does refuse outright — there is nothing left to preview by then.
    pub preview_only: bool,
}

impl Refusal {
    fn read_only(state: &State, preview_only: bool) -> Option<Refusal> {
        state.read_only.map(|reason| Refusal {
            reason,
            preview_only,
        })
    }

    /// The reason text shown dimmed next to a refused row.
    pub fn text(&self) -> String {
        if self.preview_only {
            format!("read-only ({}) · preview only", self.reason.label())
        } else {
            format!("read-only ({})", self.reason.label())
        }
    }
}

/// Which [`HelpContext`] the current state is in — the same precedence
/// [`update::mode`] uses, so a keypress and the help it describes can never
/// disagree about which mode is in force.
pub fn context(state: &State) -> HelpContext {
    match mode_beneath_help(state) {
        Mode::Help => unreachable!("mode_beneath_help never returns Help"),
        Mode::Confirm => HelpContext::Confirm,
        Mode::Editing => HelpContext::Editor(editor_context(state)),
        Mode::Filtering => HelpContext::Filter,
        Mode::Renaming => HelpContext::Rename,
        Mode::PubSubAdding => HelpContext::PubSubAdding,
        Mode::Normal => {
            // A pending chord outranks everything else in Normal mode — the
            // same "the very next keypress is spoken for" reasoning that
            // ranks `Mode::Confirm` first among `mode`'s own precedence, one
            // level down.
            if state.pending_chord.is_some() {
                return HelpContext::ChordPending;
            }
            // A full-screen view outranks the two-pane browser's own
            // Keys/Value split — it is neither.
            if state.screen == crate::state::View::Slowlog {
                return HelpContext::Slowlog;
            }
            if state.screen == crate::state::View::Monitor {
                return HelpContext::Monitor;
            }
            if state.screen == crate::state::View::PubSub {
                return HelpContext::PubSub {
                    focus: state.pubsub.focus,
                };
            }
            if state.screen == crate::state::View::Dashboard {
                // The raw-`INFO` overlay outranks the grid underneath it —
                // the same "nearest thing first" precedence `cancel()`
                // (`update/mod.rs`) uses to decide what a bare `Esc` closes.
                return if state.dashboard.active().expanded_tile.is_some() {
                    HelpContext::DashboardOverlay
                } else if state.on_cluster() && !state.dashboard.drilled() {
                    HelpContext::DashboardCluster
                } else {
                    HelpContext::Dashboard
                };
            }
            // The pane *viewed* by help, not necessarily the one actually
            // focused — `Tab` flips `HelpView::pane` while help is open
            // without moving `State::focus` (`update::help_key`'s own
            // comment), so this reads that override first, falling back to
            // real focus everywhere else (help closed, or not yet opened).
            let viewed = state.help.map_or(state.focus, |v| v.pane);
            if viewed == Pane::Keys {
                HelpContext::Keys {
                    tree: state.tree_mode,
                    filtered: !state.list.filter.is_empty(),
                }
            } else {
                HelpContext::Value(value_context(state))
            }
        }
    }
}

fn editor_context(state: &State) -> EditorContext {
    // `Mode::Editing` (above) is exactly the condition this `expect` relies
    // on: `update::mode_beneath_help` only returns it when this buffer
    // exists.
    let editor = state
        .open
        .as_ref()
        .and_then(OpenKey::typing)
        .expect("HelpContext::Editor implies an open editor buffer (Mode::Editing)");
    match editor.target() {
        EditTarget::Value => EditorContext::Value,
        EditTarget::HashField { .. } | EditTarget::ListElement { .. } => EditorContext::Field,
        // A Set member capture has no `FieldPart` at all (D3, ADR-0016) —
        // `active_part()` is `None` for it exactly the way an existing
        // field's edit is, so it stages the same way (`hint_bar`'s final
        // catch-all arm covers both).
        EditTarget::NewSetMember => EditorContext::Field,
        EditTarget::NewHashField { part, .. } | EditTarget::NewZSetMember { part, .. } => {
            let zset = matches!(editor.target(), EditTarget::NewZSetMember { .. });
            match part {
                FieldPart::Name => EditorContext::AddFormName { zset },
                FieldPart::Value => EditorContext::AddFormValue { zset },
            }
        }
        EditTarget::NewListElement { .. } => EditorContext::ListAdd,
        EditTarget::ZSetScore { .. } => EditorContext::ZSetScore,
        EditTarget::Ttl { .. } => EditorContext::Ttl,
    }
}

fn value_context(state: &State) -> Option<ValueContext> {
    let open = state.open.as_ref()?;
    let value = open.value.as_ref()?;
    let cursor = open.cursor_active;
    Some(match value {
        Value::Str(_) => ValueContext::Str { cursor },
        Value::Hash(_) => ValueContext::Hash { cursor },
        Value::List(_) => ValueContext::List { cursor },
        Value::Set(_) => ValueContext::Set { cursor },
        Value::ZSet(_) => ValueContext::ZSet { cursor },
        Value::Stream(_) => ValueContext::Stream { cursor },
        Value::Json(_) => ValueContext::Json { cursor },
        Value::Binary(_) => ValueContext::Binary { cursor },
    })
}

/// The effective binding for a plain, rebindable `Action` — `Keymap::hint`'s
/// own single-winner rule (first binding found wins, `Keymap::bind`'s doc
/// comment), the same thing every other hint in the app already shows.
/// `Action::Help` is the one action with two bindings that are both meant to
/// stay active at once (`?` and `F1`), so it does not go through this —
/// see `help_keys`, below.
fn keys_for(state: &State, action: Action) -> String {
    state.keymap.hint(action).unwrap_or_default()
}

/// A row for a plain, rebindable `Action` with an explicit label.
fn action_row(state: &State, action: Action, label: &'static str) -> HelpRow {
    HelpRow::new(keys_for(state, action), label)
}

/// `Action::Help`'s effective binding(s) in Normal mode — both `?` and `F1`
/// merged into one row when both are still bound to it (aliases merged into
/// one row, the same convention motion rows use for `↑↓ jk`), since both
/// really do open help there. Outside Normal mode only `F1` does
/// (`f1_help_keys`), so this is Normal-mode-only.
fn help_keys(state: &State) -> String {
    state
        .keymap
        .bindings()
        .iter()
        .filter(|b| b.action == Action::Help)
        .map(|b| key_label(&b.key))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The literal `F1` binding(s), filtered to just the ones spelled `F1` —
/// the only key that reaches the Editor/Filter/Confirm modes' help
/// shortcut. `?` is bound to the same `Action::Help` but is consumed as text
/// (Filter, Editor) or ignored outright (Confirm) in every mode but Normal,
/// so listing it here would show a key that does nothing (PLAN's decision
/// 5: "`?` keeps working wherever it does today (Normal mode)").
fn f1_help_keys(state: &State) -> String {
    state
        .keymap
        .bindings()
        .iter()
        .filter(|b| b.action == Action::Help && matches!(b.key.code, crate::msg::KeyCode::F(1)))
        .map(|b| key_label(&b.key))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The rows for the focused context, in rank order (most-used verbs first,
/// motions last — PLAN's "Rank order per context").
pub fn here(state: &State, ctx: HelpContext) -> Vec<HelpRow> {
    match ctx {
        HelpContext::Keys { tree, filtered } => keys_rows(state, tree, filtered),
        HelpContext::Filter => vec![
            HelpRow::new(keys_for(state, Action::EnterValueCursor), "apply"),
            // Esc's wording here is `filter_key`'s own — it clears the
            // pattern *and* exits, unlike every other Esc in the app, which
            // only backs out one layer.
            HelpRow::new(keys_for(state, Action::Cancel), "clear & exit"),
        ],
        HelpContext::Value(open) => value_rows(state, open),
        HelpContext::Editor(ectx) => editor_rows(state, ectx),
        HelpContext::Confirm => confirm_rows(state),
        HelpContext::ChordPending => chord_pending_rows(state),
        HelpContext::Slowlog => slowlog_rows(state),
        HelpContext::Monitor => monitor_rows(state),
        HelpContext::Rename => vec![
            HelpRow::new(keys_for(state, Action::EnterValueCursor), "stage rename"),
            HelpRow::new(keys_for(state, Action::Cancel), "cancel"),
        ],
        HelpContext::PubSubAdding => vec![
            HelpRow::new(keys_for(state, Action::EnterValueCursor), "subscribe"),
            HelpRow::new(
                "Tab",
                if state.sharded_available() {
                    "sharded"
                } else {
                    "sharded: needs Redis 7"
                },
            ),
            HelpRow::new(keys_for(state, Action::Cancel), "cancel"),
        ],
        HelpContext::PubSub { focus } => pubsub_rows(state, focus),
        HelpContext::Dashboard => dashboard_rows(state),
        HelpContext::DashboardCluster => dashboard_cluster_rows(state),
        HelpContext::DashboardOverlay => dashboard_overlay_rows(state),
    }
}

/// Every chord's continuation, while its prefix is pending — `k keys`,
/// `s slowlog` — read straight off [`crate::keymap::Keymap::chords`] rather
/// than hard-coded, so a rebound second key (once chords can be rebound past
/// phase A) shows here too (R7.5).
fn chord_pending_rows(state: &State) -> Vec<HelpRow> {
    state
        .keymap
        .chords()
        .iter()
        .map(|c| HelpRow::new(key_label(&c.second), c.action.label()))
        .collect()
}

/// The Slowlog view's own rows (PLAN decision 3: movement, `s`, `r`, `c`,
/// `d`).
fn slowlog_rows(state: &State) -> Vec<HelpRow> {
    vec![
        HelpRow::new(keys_for(state, Action::Sort), "sort"),
        // Never "reconnect" is wrong here either: a dropped link has nothing
        // to fetch any more than the Viewer does, so this reads the same
        // `refetch_label` every other view already shares.
        HelpRow::new(refetch_keys(state), refetch_label(state, false)),
        HelpRow::new(keys_for(state, Action::Copy), "copy command"),
        // `d`: stages `SLOWLOG RESET` (M3 phase B, Decision 3) — dimmed
        // `· preview only` under Read-only Mode exactly like every other
        // mutation-starting key (`mutation_entry_row`'s own rule), even
        // though this one has no key of its own to refuse a write against.
        mutation_entry_row(
            state,
            keys_for(state, Action::Delete),
            if state.on_cluster() {
                "reset slowlog (every node)"
            } else {
                "reset slowlog"
            },
        ),
        HelpRow::new("↑↓ jk", "move"),
        HelpRow::new("PgUp/PgDn", "page"),
        HelpRow::new("Home/End", "top/bottom"),
    ]
}

/// The Monitor view's own rows (M3 phase B, `docs/plans/m3-monitor.md`
/// decisions 5, 6, 8, 9): pause/resume, filter, reopen (only worth a row
/// once the feed actually needs it), copy, and movement — `End` doubles as
/// "resume following" (decision 7), worth saying here since it is not
/// otherwise obvious from the label `Action::Bottom` carries everywhere
/// else.
fn monitor_rows(state: &State) -> Vec<HelpRow> {
    let mut rows = Vec::new();
    // Pause/resume means nothing with no feed actually streaming — offered
    // only while `Open`, matching the guard `update::monitor::monitor_dispatch`
    // puts on the keypress itself (connecting or closed, `p` is a no-op, so
    // the hint bar must not advertise it as though it did something).
    if state.monitor.status == crate::state::FeedStatus::Open {
        let pause_label = if state.monitor.paused {
            "resume"
        } else {
            "pause"
        };
        rows.push(HelpRow::new(
            keys_for(state, Action::TogglePause),
            pause_label,
        ));
    }
    rows.push(HelpRow::new(
        keys_for(state, Action::Filter),
        if state.monitor.filter.is_empty() {
            "filter"
        } else {
            "change filter"
        },
    ));
    if !matches!(
        state.monitor.status,
        crate::state::FeedStatus::Connecting | crate::state::FeedStatus::Open
    ) {
        rows.push(HelpRow::new(keys_for(state, Action::Refetch), "reopen"));
    }
    rows.push(HelpRow::new(keys_for(state, Action::Copy), "copy command"));
    rows.push(HelpRow::new("↑↓ jk", "move"));
    rows.push(HelpRow::new("PgUp/PgDn", "page"));
    rows.push(HelpRow::new("Home/End", "top/bottom (End resumes follow)"));
    rows
}

/// The Pub/Sub view's own rows (M3 task 5, `docs/plans/m3-pubsub.md`
/// decision 1): `Tab` always offered first (it is how the reader reaches
/// every row beneath it), then strip-scoped or tail-scoped rows depending
/// on `focus`, then `r reopen` whenever there is a closed, non-empty feed to
/// reopen (offered regardless of focus, mirroring `update::pubsub::pubsub_dispatch`'s
/// own `Action::Refetch` guard, which is not focus-gated).
fn pubsub_rows(state: &State, focus: crate::state::PubSubFocus) -> Vec<HelpRow> {
    // `a add` from either half, matching `update::pubsub::pubsub_dispatch`,
    // which no longer gates `Action::Add` on focus.
    let mut rows = vec![
        HelpRow::new(keys_for(state, Action::CyclePane), "focus strip/tail"),
        HelpRow::new(keys_for(state, Action::Add), "add"),
    ];
    match focus {
        crate::state::PubSubFocus::Strip => {
            // Chip navigation/removal means nothing with no chips — offered
            // only once there is at least one, matching the guard
            // `update::pubsub::move_chip`/`remove_subscription_at` put on
            // the keypress itself.
            if !state.pubsub.subscriptions.is_empty() {
                rows.push(HelpRow::new(keys_for(state, Action::Delete), "unsubscribe"));
                rows.push(HelpRow::new("←→", "select chip"));
            }
        }
        crate::state::PubSubFocus::Tail => {
            // Pause/resume means nothing with no feed actually streaming —
            // the same guard `monitor_rows` puts on its own pause row.
            if state.pubsub.status == crate::state::FeedStatus::Open {
                let pause_label = if state.pubsub.paused {
                    "resume"
                } else {
                    "pause"
                };
                rows.push(HelpRow::new(
                    keys_for(state, Action::TogglePause),
                    pause_label,
                ));
            }
            rows.push(HelpRow::new(
                keys_for(state, Action::Filter),
                if state.pubsub.filter.is_empty() {
                    "filter"
                } else {
                    "change filter"
                },
            ));
            rows.push(HelpRow::new(keys_for(state, Action::Copy), "copy payload"));
            rows.push(HelpRow::new("↑↓ jk", "move"));
            rows.push(HelpRow::new("PgUp/PgDn", "page"));
            rows.push(HelpRow::new("Home/End", "top/bottom (End resumes follow)"));
        }
    }
    if matches!(state.pubsub.status, crate::state::FeedStatus::Closed { .. })
        && !state.pubsub.subscriptions.is_empty()
    {
        rows.push(HelpRow::new(keys_for(state, Action::Refetch), "reopen"));
    }
    rows
}

/// The Dashboard grid's own rows (M3 task 6, decision 6): tile-focus
/// movement, expanding the focused tile, refetch, and copying its raw
/// `INFO` section.
fn dashboard_rows(state: &State) -> Vec<HelpRow> {
    vec![
        HelpRow::new("←→↑↓ hjkl", "move focus"),
        HelpRow::new(keys_for(state, Action::EnterValueCursor), "expand tile"),
        HelpRow::new(refetch_keys(state), refetch_label(state, false)),
        HelpRow::new(keys_for(state, Action::Copy), "copy section"),
    ]
}

/// The Cluster Dashboard's node table (M5 task 7, decision 4): the cursor,
/// opening a node, and polling now. Scoped to the view, no new top-level key
/// (DESIGN §4, ADR-0020).
fn dashboard_cluster_rows(state: &State) -> Vec<HelpRow> {
    vec![
        HelpRow::new("↑↓ jk", "move node"),
        HelpRow::new(keys_for(state, Action::EnterValueCursor), "open node"),
        HelpRow::new(refetch_keys(state), refetch_label(state, false)),
    ]
}

/// The Dashboard's raw-`INFO` overlay's own rows (decision 6): scrolling,
/// copying the section it is showing, refetching without closing it first
/// (`update::dashboard::dashboard_dispatch`'s own overlay branch answers
/// `r` the same as the grid does), and closing it.
fn dashboard_overlay_rows(state: &State) -> Vec<HelpRow> {
    vec![
        HelpRow::new("↑↓ jk", "scroll"),
        HelpRow::new(keys_for(state, Action::Copy), "copy section"),
        HelpRow::new(refetch_keys(state), refetch_label(state, false)),
        HelpRow::new(keys_for(state, Action::Cancel), "close"),
    ]
}

fn keys_rows(state: &State, tree: bool, filtered: bool) -> Vec<HelpRow> {
    let mut rows = vec![
        HelpRow::new(keys_for(state, Action::Open), "open / expand"),
        HelpRow::new(
            keys_for(state, Action::Filter),
            if filtered { "change filter" } else { "filter" },
        ),
        HelpRow::new(keys_for(state, Action::Sort), "sort"),
        HelpRow::new(
            keys_for(state, Action::ToggleTree),
            if tree { "flat view" } else { "tree view" },
        ),
    ];
    let delete = HelpRow::new(keys_for(state, Action::Delete), "delete");
    rows.push(match Refusal::read_only(state, true) {
        Some(r) => delete.refused(r),
        None => delete,
    });
    rows.push(HelpRow::new(keys_for(state, Action::Copy), "copy"));
    rows.push(HelpRow::new(
        refetch_keys(state),
        refetch_label(state, true),
    ));
    // Ranked after `r`, not beside `c`: its label is the longest in the
    // pane, and the hint bar stops at the first row that does not fit, so
    // ranked earlier it would push `r` (and `t`) off the bar at 80 columns.
    rows.push(HelpRow::new(
        keys_for(state, Action::CopyCommand),
        "copy redis-cli command",
    ));
    // Ranked last among the verbs, for the same reason: a new row must not push
    // `r` or `t` off an 80-column bar. Dimmed under Read-only Mode like delete.
    rows.push(mutation_entry_row(
        state,
        keys_for(state, Action::Rename),
        "rename",
    ));
    rows.push(HelpRow::new("↑↓ jk", "move"));
    rows.push(HelpRow::new("PgUp/PgDn", "page"));
    rows.push(HelpRow::new("Home/End", "top/bottom"));
    rows
}

/// `r`'s effective binding, in either pane — one Action, two verbs
/// (`Action::Refetch`'s own doc comment).
fn refetch_keys(state: &State) -> String {
    keys_for(state, Action::Refetch)
}

/// `r`'s label: `reconnect` over a dropped link, `rescan`/`refetch`
/// otherwise, matching exactly what `update::refetch_action` does — not the
/// broader `Liveness::Disconnected` the old `hint_bar` read this off of,
/// which also covers `Link::Connecting` (real startup never renders an
/// interactive frame there; `refetch_action`'s own comment is explicit that
/// this check is deliberately narrower).
fn refetch_label(state: &State, keys_pane: bool) -> &'static str {
    if matches!(state.link, Link::Reconnecting { .. }) {
        "reconnect"
    } else if keys_pane {
        "rescan"
    } else {
        "refetch"
    }
}

/// A row that opens the path toward a mutation (opening the editor, or
/// staging one directly) — dimmed with `preview only` under Read-only Mode
/// the same way the confirm dialog's `y` eventually would be, so the reader
/// learns the outcome before typing a whole value rather than after
/// (coordinator note, phase A review: "the reader would otherwise type a
/// whole value before learning it will be refused"). Read-only Mode itself
/// never blocks any of these keypresses (`update::mode`'s comment — the
/// refusal is only real at `y`), so this is *anticipatory* dimming, not a
/// report of what the keypress does right now.
fn mutation_entry_row(state: &State, keys: String, label: &'static str) -> HelpRow {
    let row = HelpRow::new(keys, label);
    match Refusal::read_only(state, true) {
        Some(r) => row.refused(r),
        None => row,
    }
}

fn value_rows(state: &State, open: Option<ValueContext>) -> Vec<HelpRow> {
    let Some(open) = open else {
        // Nothing read yet — Copy/Add/Edit/Delete all refuse with "nothing
        // open"/"open a key first" notices (`open_editor`, `begin_add_entry`,
        // `delete_value_row`'s own early returns); there is nothing here to
        // offer a row for. `status` carries the notice text.
        return Vec::new();
    };
    let mut rows = Vec::new();
    // `Action::pane_is_on_screen`'s movement group reads `cursor_active`
    // regardless of type — `Enter`/`Action::EnterValueCursor` can activate it
    // for *any* type with a value (`enter_value_cursor`'s own gate is only
    // "has a value", not "is a collection") — so this decides whether plain
    // movement drives the value or the key list for every type, not only the
    // five with row-level actions.
    let cursor_active = open.cursor();
    // Whether this type has anything a cursor picks a row *of* — Hash/List/
    // Set/ZSet/Stream (`delete_value_row`'s exhaustive match, one type
    // short of Stream, which it refuses like Str/Json/Binary, but which
    // still has rows to move a cursor over for reading). Str/Json/Binary
    // have no row-level actions at all, so there is nothing `Enter` would
    // unlock for them worth advertising here.
    let has_rows = matches!(
        open,
        ValueContext::Hash { .. }
            | ValueContext::List { .. }
            | ValueContext::Set { .. }
            | ValueContext::ZSet { .. }
            | ValueContext::Stream { .. }
    );
    // `e`: String/JSON edit in place; a Hash field/List element/ZSet score
    // needs the cursor on a row first; Set and Stream and Binary never offer
    // it at all (`open_editor`'s ladder — every refusal above cites the
    // branch it mirrors).
    match open {
        ValueContext::Str { .. } | ValueContext::Json { .. } => {
            rows.push(mutation_entry_row(
                state,
                keys_for(state, Action::Edit),
                "edit",
            ));
        }
        ValueContext::Hash { .. } if cursor_active => {
            rows.push(mutation_entry_row(
                state,
                keys_for(state, Action::Edit),
                "edit field",
            ));
        }
        ValueContext::List { .. } if cursor_active => {
            rows.push(mutation_entry_row(
                state,
                keys_for(state, Action::Edit),
                "edit element",
            ));
        }
        // ZSet's `e` edits the score, never the member (D1, ADR-0018) — the
        // label says `score`, not `edit`: D1's own text is explicit that
        // this is not a surprise to spell out ("the hint says score, not
        // edit"), and the pre-existing `hint_bar` test
        // `a_zset_with_the_cursor_active_hints_score_not_edit` enforces the
        // word "edit" never appearing on this row.
        ValueContext::ZSet { .. } if cursor_active => {
            rows.push(mutation_entry_row(
                state,
                keys_for(state, Action::Edit),
                "score",
            ));
        }
        // Set: `e` always refuses (a member's bytes are its whole identity —
        // D1, ADR-0016), so there is no row to offer at all, not a dimmed
        // one — the same "not applicable" omission the old `hint_bar` used
        // for this exact case. Binary/Stream: never editable in place
        // either (`EditBuffer::from_value`'s refusal for Binary; Stream
        // falls through the same match with no branch of its own).
        _ => {}
    }
    // `a`: Hash/Set/List/ZSet only (`begin_add_entry`'s exhaustive match);
    // no cursor prerequisite — a new entry has no row yet to have picked.
    let add_label = match open {
        ValueContext::Hash { .. } => Some("add field"),
        ValueContext::Set { .. } => Some("add member"),
        ValueContext::List { .. } => Some("add element"),
        ValueContext::ZSet { .. } => Some("add member"),
        _ => None,
    };
    if let Some(label) = add_label {
        rows.push(mutation_entry_row(
            state,
            keys_for(state, Action::Add),
            label,
        ));
    }
    // `d`: Hash/Set/List/ZSet, cursor on a row (`delete_value_row`'s
    // exhaustive match) — stages the mutation straight away.
    let delete_label = match open {
        ValueContext::Hash { .. } if cursor_active => Some("delete field"),
        ValueContext::Set { .. } if cursor_active => Some("remove member"),
        ValueContext::List { .. } if cursor_active => Some("delete element"),
        ValueContext::ZSet { .. } if cursor_active => Some("remove member"),
        _ => None,
    };
    if let Some(label) = delete_label {
        rows.push(mutation_entry_row(
            state,
            keys_for(state, Action::Delete),
            label,
        ));
    }
    // Plain movement drives the key list, not the value, until `Enter`
    // activates the cursor (`Action::pane_is_on_screen`'s movement group) —
    // so for a type with row-level actions behind that cursor, the way in is
    // worth its own row while it is still off, immediately before the
    // motions below get relabeled to match what they actually do right now.
    if has_rows && !cursor_active {
        rows.push(HelpRow::new(
            keys_for(state, Action::EnterValueCursor),
            "move in value",
        ));
    }
    rows.push(HelpRow::new(keys_for(state, Action::Copy), "copy"));
    // `t`: every type, no cursor prerequisite (`open_ttl_editor`'s doc
    // comment: "a String, a Hash, a Stream and a binary blob all have
    // exactly one TTL, edited identically").
    rows.push(mutation_entry_row(
        state,
        keys_for(state, Action::ToggleTree),
        "edit ttl",
    ));
    rows.push(HelpRow::new(
        refetch_keys(state),
        refetch_label(state, false),
    ));
    // Ranked after `r`, not beside `c`: its label is the longest in the
    // pane, and the hint bar stops at the first row that does not fit, so
    // ranked earlier it would push `r` (and `t`) off the bar at 80 columns.
    rows.push(HelpRow::new(
        keys_for(state, Action::CopyCommand),
        "copy redis-cli command",
    ));
    if cursor_active {
        rows.push(HelpRow::new("↑↓ jk", "move"));
        rows.push(HelpRow::new("PgUp/PgDn", "page"));
        rows.push(HelpRow::new("Home/End", "top/bottom"));
    } else if has_rows {
        // Cursor off, but this type has rows to move a cursor over once it
        // is on — plain movement is still live, just aimed at the key list
        // instead (the same `pane_is_on_screen` fact the row above names),
        // so the label says what it does now rather than dropping the row.
        rows.push(HelpRow::new("↑↓ jk", "next key"));
        rows.push(HelpRow::new("PgUp/PgDn", "page keys"));
        rows.push(HelpRow::new("Home/End", "top/bottom key"));
    }
    rows
}

fn editor_rows(state: &State, ctx: EditorContext) -> Vec<HelpRow> {
    let cancel = HelpRow::new(keys_for(state, Action::Cancel), "cancel");
    let undo = HelpRow::new(keys_for(state, Action::EditorUndo), "undo");
    let stage_row = |state: &State, keys: String, label: &'static str| {
        let row = HelpRow::new(keys, label);
        match Refusal::read_only(state, true) {
            Some(r) => row.refused(r),
            None => row,
        }
    };
    match ctx {
        EditorContext::Value => vec![
            stage_row(state, keys_for(state, Action::EditorStage), "stage"),
            undo,
            cancel,
        ],
        EditorContext::Field => vec![stage_row(state, "Enter".to_string(), "stage"), undo, cancel],
        // The next half's noun differs by form (`hint_bar`'s own distinction,
        // today: "Enter value" for the Hash add form's name part, "Enter
        // score" for the ZSet add form's member part) — D6, ADR-0018.
        EditorContext::AddFormName { zset } => {
            let next = if zset { "next: score" } else { "next: value" };
            vec![HelpRow::new("Enter", next), cancel]
        }
        EditorContext::AddFormValue { zset } => {
            let back = if zset {
                "back to member"
            } else {
                "back to field"
            };
            vec![
                stage_row(state, "Enter".to_string(), "stage"),
                HelpRow::new("↑", back),
                undo,
                cancel,
            ]
        }
        EditorContext::ListAdd => vec![
            stage_row(state, "Enter".to_string(), "stage"),
            HelpRow::new("Tab", "head/tail"),
            undo,
            cancel,
        ],
        EditorContext::ZSetScore => vec![
            stage_row(state, keys_for(state, Action::EditorStage), "stage"),
            undo,
            cancel,
        ],
        EditorContext::Ttl => vec![
            stage_row(state, keys_for(state, Action::EditorStage), "apply"),
            cancel,
        ],
    }
}

fn confirm_rows(state: &State) -> Vec<HelpRow> {
    let confirm = HelpRow::new(keys_for(state, Action::ConfirmMutation), "confirm");
    let confirm = match Refusal::read_only(state, false) {
        Some(r) => confirm.refused(r),
        None => confirm,
    };
    vec![
        confirm,
        HelpRow::new(keys_for(state, Action::Cancel), "discard"),
    ]
}

/// The global rows: everywhere in Normal mode, and just `F1 help` while the
/// editor, the filter or the confirm dialog owns the keyboard —
/// `confirm_key`/`filter_key`/`editor_key` hardcode their own vocabulary
/// rather than resolving through the keymap, so `Action::Quit`/
/// `Action::ToggleReadOnly`/`Action::CyclePane` are simply unreachable there,
/// and each of the three already carries its own way back as a HERE row
/// (`here`'s `Filter`/`Editor`/`Confirm` arms), so `Action::Cancel` would
/// only repeat it here.
pub fn everywhere(state: &State) -> Vec<HelpRow> {
    match mode_beneath_help(state) {
        // `everywhere` describes the context help is drawn over, exactly
        // like `context` (above) — never "what Help mode itself allows".
        Mode::Help => unreachable!("mode_beneath_help never returns Help"),
        Mode::Normal if state.pending_chord.is_some() => {
            // A pending chord is its own tiny mode (`HelpContext::ChordPending`
            // above): the continuations are the whole of HERE, and EVERYWHERE
            // shrinks to just the way out and help — the same shape
            // Filter/Editor/Confirm give it below, one rank up. Showing the
            // ordinary Normal-mode EVERYWHERE here would repeat `g s slowlog`
            // right after `s slowlog` in the same breath, which reads as if
            // the chord had two different meanings rather than one already in
            // progress (Decision 1, `docs/plans/m3-slowlog.md`: "the hint bar
            // and help show its continuations plus `Esc cancel` ... only").
            //
            // Checked before the Dashboard-overlay arm below, matching
            // `context`'s own precedence: "a pending chord outranks
            // everything else in Normal mode."
            vec![
                HelpRow::new(keys_for(state, Action::Cancel), "cancel"),
                HelpRow::new(help_keys(state), "help"),
            ]
        }
        // The Dashboard's raw-`INFO` overlay is its own tiny mode, the same
        // shape a pending chord already gets one arm up: HERE
        // (`dashboard_overlay_rows`) already carries its own `Esc close`
        // row, so EVERYWHERE shrinks to just `F1 help` — the same
        // `Mode::Confirm`/`Mode::Editing`/`Mode::Filtering` treatment below,
        // reached here rather than there because the overlay is not its own
        // `Mode` (it is `Mode::Normal` with `state.dashboard.expanded_tile`
        // set, not a mode `key_press`'s own precedence needs to know about).
        Mode::Normal
            if state.screen == crate::state::View::Dashboard
                && state.dashboard.active().expanded_tile.is_some() =>
        {
            vec![HelpRow::new(f1_help_keys(state), "help")]
        }
        Mode::Normal => {
            let mut rows = Vec::new();
            // `Tab`/`Action::CyclePane`: only worth a row when it would move
            // focus somewhere. From the keys pane it only ever moves to the
            // Viewer when a key is open (`dispatch_action`'s own match arm);
            // from the Viewer it always goes back. Below 70 columns this is
            // also what draws the other pane at all, but it is still a
            // meaningful keypress either way, so it is not gated on width.
            // Suppressed in the Pub/Sub view: `Tab` still means "focus
            // strip/tail" there, but `pubsub_rows` already carries that row
            // as HERE content (decision 1) — a second, differently-worded
            // copy here would be a duplicate rather than new information.
            if !matches!(
                state.screen,
                crate::state::View::PubSub | crate::state::View::Dashboard
            ) && (!state.keys_pane_focused() || state.open.is_some())
            {
                rows.push(action_row(state, Action::CyclePane, "focus"));
            }
            // `⌃R`: omitted, not dimmed, under `replica` — never offer a
            // toggle the server will refuse anyway (CLAUDE.md,
            // `dispatch_action`'s own comment on `Action::ToggleReadOnly`).
            if !matches!(state.read_only, Some(ReadOnlyReason::Replica)) {
                rows.push(action_row(state, Action::ToggleReadOnly, "read-only"));
            }
            // The view chords (M3, `docs/plans/m3-slowlog.md`): reachable
            // from anywhere in Normal mode, so they belong beside `Tab`/
            // `⌃R`, not only inside `HelpContext::ChordPending`'s
            // continuation list, which only shows once `g` has already been
            // pressed.
            if let Some(keys) = state.keymap.chord_hint(Action::OpenSlowlog) {
                rows.push(HelpRow::new(keys, Action::OpenSlowlog.label()));
            }
            if let Some(keys) = state.keymap.chord_hint(Action::OpenMonitor) {
                rows.push(HelpRow::new(keys, Action::OpenMonitor.label()));
            }
            if let Some(keys) = state.keymap.chord_hint(Action::OpenPubSub) {
                rows.push(HelpRow::new(keys, Action::OpenPubSub.label()));
            }
            if let Some(keys) = state.keymap.chord_hint(Action::OpenDashboard) {
                rows.push(HelpRow::new(keys, Action::OpenDashboard.label()));
            }
            if matches!(
                state.screen,
                crate::state::View::Slowlog
                    | crate::state::View::Monitor
                    | crate::state::View::PubSub
                    | crate::state::View::Dashboard
            ) && let Some(keys) = state.keymap.chord_hint(Action::OpenKeysView)
            {
                rows.push(HelpRow::new(keys, Action::OpenKeysView.label()));
            }
            rows.push(action_row(state, Action::Cancel, "back"));
            rows.push(action_row(state, Action::Quit, "quit"));
            rows.push(HelpRow::new(help_keys(state), "help"));
            rows
        }
        // Filter/Editor/Confirm each already carry their own way back as a
        // HERE row (`clear & exit`, `cancel`, `discard`) — `confirm_key`/
        // `filter_key`/`editor_key` all handle `Esc` themselves, so a second
        // `Action::Cancel`-labelled row here would just repeat it. `F1` is
        // the only thing genuinely global left in these three modes.
        Mode::Confirm | Mode::Editing | Mode::Filtering | Mode::Renaming | Mode::PubSubAdding => {
            vec![HelpRow::new(f1_help_keys(state), "help")]
        }
    }
}

/// The bar's non-row messages — a duplicate-field/member warning, or the
/// live invalid-score indicator — that replace part of the row list rather
/// than sitting beside it (`hint_bar`'s editor branch, today).
pub fn status(state: &State) -> Option<String> {
    let editor = state.open.as_ref().and_then(OpenKey::typing)?;
    let cancel = keys_for(state, Action::Cancel);
    match editor.active_part() {
        Some(FieldPart::Name) => {
            let is_zset = matches!(editor.target(), EditTarget::NewZSetMember { .. });
            let duplicate = state.open.as_ref().is_some_and(|o| {
                if is_zset {
                    o.zset_member_shown_duplicate()
                } else {
                    o.hash_field_shown_duplicate()
                }
            });
            if !duplicate {
                return None;
            }
            let edit = keys_for(state, Action::Edit);
            Some(if is_zset {
                format!("member exists — {cancel}, then {edit} its score")
            } else {
                format!("field exists — {cancel}, then {edit} to edit")
            })
        }
        Some(FieldPart::Value) => {
            let is_zset = matches!(editor.target(), EditTarget::NewZSetMember { .. });
            if is_zset
                && !crate::state::is_valid_zset_score(&String::from_utf8_lossy(&editor.text()))
            {
                Some("invalid score".to_string())
            } else {
                None
            }
        }
        None if matches!(editor.target(), EditTarget::ZSetScore { .. }) => {
            if !crate::state::is_valid_zset_score(&String::from_utf8_lossy(&editor.text())) {
                Some("invalid score".to_string())
            } else {
                None
            }
        }
        None if matches!(editor.target(), EditTarget::Ttl { .. }) => {
            Some("never persists".to_string())
        }
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::layout::Pane;
    use crate::state::value::{PairValue, StringValue};
    use crate::state::{EditBuffer, OpenKey};

    fn opened(value: Value) -> State {
        State {
            cols: 130,
            rows: 40,
            focus: Pane::Value,
            open: Some(OpenKey::new(Some(0), "k".into(), value, -1, 10, 0)),
            ..State::default()
        }
    }

    // ── context() ────────────────────────────────────────────────────────

    #[test]
    fn keys_pane_focused_is_the_keys_context() {
        let s = State::default();
        assert_eq!(
            context(&s),
            HelpContext::Keys {
                tree: false,
                filtered: false
            }
        );
    }

    #[test]
    fn filtering_outranks_keys_context() {
        let s = State {
            filtering: true,
            ..State::default()
        };
        assert_eq!(context(&s), HelpContext::Filter);
    }

    #[test]
    fn a_staged_mutation_outranks_everything() {
        let mut s = opened(Value::Str(StringValue::new("v", 40)));
        s.filtering = true; // would be Filter, if Confirm didn't outrank it
        s.confirm = Some(crate::state::PendingMutation::DeleteKey {
            index: 0,
            name: "k".into(),
        });
        assert_eq!(context(&s), HelpContext::Confirm);
    }

    #[test]
    fn value_pane_focused_with_nothing_open_is_none() {
        let s = State {
            focus: Pane::Value,
            ..State::default()
        };
        assert_eq!(context(&s), HelpContext::Value(None));
    }

    #[test]
    fn value_pane_reports_type_and_cursor() {
        let mut s = opened(Value::Hash(PairValue::default()));
        assert_eq!(
            context(&s),
            HelpContext::Value(Some(ValueContext::Hash { cursor: false }))
        );
        s.open.as_mut().unwrap().cursor_active = true;
        assert_eq!(
            context(&s),
            HelpContext::Value(Some(ValueContext::Hash { cursor: true }))
        );
    }

    #[test]
    fn editor_open_outranks_value_context() {
        let mut s = opened(Value::Str(StringValue::new("v", 40)));
        s.open
            .as_mut()
            .unwrap()
            .begin_edit(EditBuffer::from_value(&Value::Str(StringValue::new("v", 40)), 0).unwrap());
        assert_eq!(context(&s), HelpContext::Editor(EditorContext::Value));
    }

    // ── refusals ─────────────────────────────────────────────────────────

    #[test]
    fn replica_omits_ctrl_r_from_everywhere() {
        let s = State {
            read_only: Some(ReadOnlyReason::Replica),
            ..State::default()
        };
        let rows = everywhere(&s);
        assert!(rows.iter().all(|r| r.label != "read-only"), "{rows:?}");
    }

    #[test]
    fn environment_read_only_keeps_ctrl_r_and_does_not_omit_it() {
        let s = State {
            read_only: Some(ReadOnlyReason::Environment),
            ..State::default()
        };
        let rows = everywhere(&s);
        assert!(rows.iter().any(|r| r.label == "read-only"), "{rows:?}");
    }

    #[test]
    fn read_only_dims_the_keys_pane_delete_row_with_preview_only() {
        let s = State {
            read_only: Some(ReadOnlyReason::Environment),
            ..State::default()
        };
        let rows = here(&s, context(&s));
        let delete = rows
            .iter()
            .find(|r| r.label == "delete")
            .expect("delete row");
        assert_eq!(
            delete.refused.map(|r| r.text()),
            Some("read-only (environment) · preview only".to_string())
        );
    }

    #[test]
    fn read_only_dims_the_hash_field_delete_row_with_preview_only() {
        let mut s = opened(Value::Hash(PairValue {
            pairs: vec![(b"f".to_vec(), b"v".to_vec())],
            total: 1,
        }));
        s.open.as_mut().unwrap().cursor_active = true;
        s.read_only = Some(ReadOnlyReason::User);
        let rows = here(&s, context(&s));
        let delete = rows
            .iter()
            .find(|r| r.label == "delete field")
            .expect("delete field row");
        assert_eq!(
            delete.refused.map(|r| r.text()),
            Some("read-only (user) · preview only".to_string())
        );
    }

    /// Coordinator note, phase A review: every key that *starts* a mutation
    /// gets the same `preview only` dimming `d` already had — `e`, `a`, and
    /// `t` meaning edit ttl — not only the row that stages it outright, so
    /// the reader learns the refusal before typing a whole value into an
    /// editor the confirm dialog will refuse to run.
    #[test]
    fn read_only_dims_edit_add_and_edit_ttl_rows_in_the_value_pane() {
        let mut s = opened(Value::Hash(PairValue {
            pairs: vec![(b"f".to_vec(), b"v".to_vec())],
            total: 1,
        }));
        s.open.as_mut().unwrap().cursor_active = true;
        s.read_only = Some(ReadOnlyReason::Environment);
        let rows = here(&s, context(&s));
        for label in ["edit field", "add field", "edit ttl"] {
            let row = rows.iter().find(|r| r.label == label).unwrap_or_else(|| {
                panic!("no {label:?} row in {rows:?}");
            });
            assert_eq!(
                row.refused.map(|r| r.text()),
                Some("read-only (environment) · preview only".to_string()),
                "{label} row"
            );
        }
    }

    #[test]
    fn without_read_only_the_edit_add_and_ttl_rows_are_not_dimmed() {
        let mut s = opened(Value::Hash(PairValue {
            pairs: vec![(b"f".to_vec(), b"v".to_vec())],
            total: 1,
        }));
        s.open.as_mut().unwrap().cursor_active = true;
        let rows = here(&s, context(&s));
        for label in ["edit field", "add field", "edit ttl"] {
            let row = rows.iter().find(|r| r.label == label).expect("row");
            assert!(row.refused.is_none(), "{label} row: {row:?}");
        }
    }

    // ── cursor off redirects motion to the key list (coordinator note,
    // phase A review) ───────────────────────────────────────────────────

    #[test]
    fn cursor_off_on_a_hash_offers_enter_and_relabels_motion_to_the_key_list() {
        let s = opened(Value::Hash(PairValue {
            pairs: vec![(b"f".to_vec(), b"v".to_vec())],
            total: 1,
        }));
        assert!(!s.open.as_ref().unwrap().cursor_active);
        let rows = here(&s, context(&s));
        assert!(rows.iter().any(|r| r.label == "move in value"), "{rows:?}");
        assert!(rows.iter().any(|r| r.label == "next key"), "{rows:?}");
        assert!(rows.iter().any(|r| r.label == "page keys"), "{rows:?}");
        assert!(rows.iter().any(|r| r.label == "top/bottom key"), "{rows:?}");
        assert!(!rows.iter().any(|r| r.label == "move"));
    }

    #[test]
    fn cursor_on_a_hash_drops_the_enter_row_and_labels_motion_as_move() {
        let mut s = opened(Value::Hash(PairValue {
            pairs: vec![(b"f".to_vec(), b"v".to_vec())],
            total: 1,
        }));
        s.open.as_mut().unwrap().cursor_active = true;
        let rows = here(&s, context(&s));
        assert!(!rows.iter().any(|r| r.label == "move in value"));
        assert!(rows.iter().any(|r| r.label == "move"), "{rows:?}");
        assert!(!rows.iter().any(|r| r.label == "next key"));
    }

    #[test]
    fn a_type_with_no_row_level_actions_never_offers_move_in_value() {
        // Str has nothing a cursor picks a row of — `has_rows` is false, so
        // there is nothing for `Enter` to unlock worth advertising, cursor on
        // or off.
        let s = opened(Value::Str(StringValue::new("v", 40)));
        let rows = here(&s, context(&s));
        assert!(!rows.iter().any(|r| r.label == "move in value"), "{rows:?}");
        assert!(!rows.iter().any(|r| r.label == "next key"), "{rows:?}");
    }

    #[test]
    fn read_only_refuses_the_confirm_y_row_outright_no_preview_only_suffix() {
        let mut s = opened(Value::Str(StringValue::new("v", 40)));
        s.confirm = Some(crate::state::PendingMutation::DeleteKey {
            index: 0,
            name: "k".into(),
        });
        s.read_only = Some(ReadOnlyReason::Environment);
        let rows = here(&s, context(&s));
        let confirm = rows
            .iter()
            .find(|r| r.label == "confirm")
            .expect("confirm row");
        assert_eq!(
            confirm.refused.map(|r| r.text()),
            Some("read-only (environment)".to_string())
        );
    }

    #[test]
    fn without_read_only_no_row_is_refused() {
        let s = State::default();
        assert!(here(&s, context(&s)).iter().all(|r| r.refused.is_none()));
        assert!(everywhere(&s).iter().all(|r| r.refused.is_none()));
    }

    // ── disconnected ─────────────────────────────────────────────────────

    #[test]
    fn disconnected_shows_reconnect_in_the_keys_pane() {
        let s = State {
            link: Link::Reconnecting {
                attempt: 1,
                retry_in_ms: None,
            },
            ..State::default()
        };
        let rows = here(&s, context(&s));
        assert!(rows.iter().any(|r| r.label == "reconnect"), "{rows:?}");
        assert!(!rows.iter().any(|r| r.label == "rescan"));
    }

    #[test]
    fn connecting_at_startup_still_says_rescan_not_reconnect() {
        // `Link::Connecting` is `Liveness::Disconnected` too, but
        // `refetch_action` only retries over `Link::Reconnecting` — a
        // dropped link, never the pre-first-connect state (its own comment
        // says real startup never renders an interactive frame there, but
        // `State::default()` is exactly this state and many tests build off
        // it, so this must not claim "reconnect" here).
        let s = State::default();
        let rows = here(&s, context(&s));
        assert!(rows.iter().any(|r| r.label == "rescan"), "{rows:?}");
    }

    // ── rebinding ────────────────────────────────────────────────────────

    #[test]
    fn a_user_rebinding_shows_in_here_and_everywhere() {
        let mut s = State::default();
        s.keymap.bind(
            Action::Sort,
            crate::msg::KeyPress::plain(crate::msg::KeyCode::Char('x')),
        );
        let rows = here(&s, context(&s));
        let sort = rows.iter().find(|r| r.label == "sort").expect("sort row");
        assert_eq!(sort.keys, "x");

        let mut s2 = State::default();
        s2.keymap.bind(
            Action::Quit,
            crate::msg::KeyPress::plain(crate::msg::KeyCode::Char('x')),
        );
        let everywhere_rows = everywhere(&s2);
        let quit = everywhere_rows
            .iter()
            .find(|r| r.label == "quit")
            .expect("quit row");
        assert_eq!(quit.keys, "x");
    }

    // ── row-count bound at 80×24 ─────────────────────────────────────────
    //
    // 24 rows total; chrome takes 5 (border top, title, a blank, the
    // headings line, border bottom) plus a 1-row footer pointer line, and
    // the EVERYWHERE block packs onto a single row of its own (PLAN:
    // "may pack several short rows per line to save height") — 24 - 5 - 1
    // (footer) - 1 (everywhere, packed) = 17 rows left for HERE.
    const HERE_ROW_BUDGET: usize = 17;

    /// Every [`HelpContext`] variant, not a sample of states that happen to
    /// produce one — `here` only ever reads `state.read_only`/`keymap`/
    /// `link` beyond `ctx` itself (`value_rows`/`editor_rows`/`confirm_rows`
    /// take everything else from `ctx`), so a plain default `State` paired
    /// with each constructed context exercises every row list `here` can
    /// produce, without needing a real `OpenKey` to back each one.
    fn every_context() -> Vec<HelpContext> {
        let mut ctxs = Vec::new();
        for tree in [false, true] {
            for filtered in [false, true] {
                ctxs.push(HelpContext::Keys { tree, filtered });
            }
        }
        ctxs.push(HelpContext::Filter);
        ctxs.push(HelpContext::Value(None));
        for cursor in [false, true] {
            for vc in [
                ValueContext::Str { cursor },
                ValueContext::Hash { cursor },
                ValueContext::List { cursor },
                ValueContext::Set { cursor },
                ValueContext::ZSet { cursor },
                ValueContext::Stream { cursor },
                ValueContext::Json { cursor },
                ValueContext::Binary { cursor },
            ] {
                ctxs.push(HelpContext::Value(Some(vc)));
            }
        }
        for ectx in [
            EditorContext::Value,
            EditorContext::Field,
            EditorContext::ListAdd,
            EditorContext::ZSetScore,
            EditorContext::Ttl,
        ] {
            ctxs.push(HelpContext::Editor(ectx));
        }
        for zset in [false, true] {
            ctxs.push(HelpContext::Editor(EditorContext::AddFormName { zset }));
            ctxs.push(HelpContext::Editor(EditorContext::AddFormValue { zset }));
        }
        ctxs.push(HelpContext::Confirm);
        ctxs.push(HelpContext::ChordPending);
        ctxs.push(HelpContext::Slowlog);
        ctxs.push(HelpContext::Dashboard);
        ctxs.push(HelpContext::DashboardCluster);
        ctxs.push(HelpContext::DashboardOverlay);
        ctxs
    }

    #[test]
    fn the_server_views_on_a_cluster_are_not_dimmed_and_the_dashboard_has_its_own_rows() {
        use crate::state::{Topology, View};
        let mut state = State {
            screen: View::Dashboard,
            ..State::default()
        };
        state.connection.topology = Some(Topology {
            primaries: 3,
            nodes: 6,
        });
        assert_eq!(context(&state), HelpContext::DashboardCluster);
        let rows = here(&state, HelpContext::DashboardCluster);
        let labels: Vec<_> = rows.iter().map(|r| &*r.label).collect();
        assert_eq!(labels, ["move node", "open node", "refetch"]);
        assert!(rows.iter().all(|r| r.refused.is_none()));
        // Slowlog and Monitor are per node too, and nothing is dimmed there.
        for (screen, ctx) in [
            (View::Slowlog, HelpContext::Slowlog),
            (View::Monitor, HelpContext::Monitor),
        ] {
            state.screen = screen;
            assert!(here(&state, ctx).iter().all(|r| r.refused.is_none()));
        }
    }

    #[test]
    fn every_context_is_covered_by_the_row_budget_sweep() {
        // A change to either enum that isn't reflected in `every_context`
        // should fail loudly here rather than silently under-testing a new
        // variant — 4 (Keys) + 1 (Filter) + 1 (Value(None)) + 16 (8 types ×
        // cursor) + 9 (5 plain Editor + 2 zset × 2 AddForm) + 1 (Confirm) +
        // 1 (ChordPending) + 1 (Slowlog) + 1 (Dashboard) + 1 (DashboardCluster) + 1 (DashboardOverlay).
        assert_eq!(every_context().len(), 37);
    }

    #[test]
    fn no_context_exceeds_the_80x24_row_budget() {
        let plain = State::default();
        let read_only = State {
            read_only: Some(ReadOnlyReason::Environment),
            ..State::default()
        };
        for ctx in every_context() {
            for s in [&plain, &read_only] {
                let rows = here(s, ctx);
                assert!(
                    rows.len() <= HERE_ROW_BUDGET,
                    "{ctx:?} (read_only={:?}) produced {} HERE rows",
                    s.read_only,
                    rows.len()
                );
            }
        }
    }

    // ── hint bar is a prefix of help (spot check; full sweep is render's
    // job once hint_bar reads from here in phase B) ─────────────────────

    #[test]
    fn status_is_none_outside_the_editor() {
        assert_eq!(status(&State::default()), None);
    }

    // ── M3: the Slowlog view and pending chords ─────────────────────────

    #[test]
    fn a_pending_chord_outranks_the_keys_context() {
        let s = State {
            pending_chord: Some(crate::msg::KeyPress::plain(crate::msg::KeyCode::Char('g'))),
            ..State::default()
        };
        assert_eq!(context(&s), HelpContext::ChordPending);
    }

    #[test]
    fn chord_pending_rows_name_both_continuations() {
        let s = State {
            pending_chord: Some(crate::msg::KeyPress::plain(crate::msg::KeyCode::Char('g'))),
            ..State::default()
        };
        let rows = here(&s, context(&s));
        assert!(
            rows.iter().any(|r| r.keys == "k" && r.label == "keys"),
            "{rows:?}"
        );
        assert!(
            rows.iter().any(|r| r.keys == "s" && r.label == "slowlog"),
            "{rows:?}"
        );
    }

    #[test]
    fn the_slowlog_view_outranks_the_keys_value_split() {
        let s = State {
            screen: crate::state::View::Slowlog,
            ..State::default()
        };
        assert_eq!(context(&s), HelpContext::Slowlog);
    }

    #[test]
    fn slowlog_rows_offer_d_as_reset_slowlog() {
        let s = State {
            screen: crate::state::View::Slowlog,
            ..State::default()
        };
        let rows = here(&s, context(&s));
        let reset = rows
            .iter()
            .find(|r| r.label == "reset slowlog")
            .expect("d row");
        assert!(reset.refused.is_none());
    }

    #[test]
    fn read_only_dims_reset_slowlog_with_preview_only() {
        let s = State {
            screen: crate::state::View::Slowlog,
            read_only: Some(ReadOnlyReason::Environment),
            ..State::default()
        };
        let rows = here(&s, context(&s));
        let reset = rows
            .iter()
            .find(|r| r.label == "reset slowlog")
            .expect("d row");
        assert_eq!(
            reset.refused.map(|r| r.text()),
            Some("read-only (environment) · preview only".to_string())
        );
    }

    #[test]
    fn everywhere_always_offers_g_s_and_only_offers_g_k_from_the_slowlog_view() {
        let keys_view = State::default();
        let rows = everywhere(&keys_view);
        assert!(rows.iter().any(|r| r.keys == "g s"), "{rows:?}");
        assert!(!rows.iter().any(|r| r.keys == "g k"), "{rows:?}");

        let slowlog_view = State {
            screen: crate::state::View::Slowlog,
            ..State::default()
        };
        let rows = everywhere(&slowlog_view);
        assert!(rows.iter().any(|r| r.keys == "g s"), "{rows:?}");
        assert!(rows.iter().any(|r| r.keys == "g k"), "{rows:?}");
    }
}
