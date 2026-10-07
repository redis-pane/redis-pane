//! Rename a key (M2 task 11, `docs/plans/m2-task11-rename.md`): the name
//! capture, the staged `RENAMENX`, its `EXISTS` pre-check, and what each
//! outcome means once the server has answered.

use super::*;
use crate::state::{NameKind, RenameCapture, SlotPath, TargetCheck};

/// `R` with the keys pane focused: open the name capture for the Selected key.
pub(super) fn begin_rename(state: State) -> (State, Vec<Command>) {
    begin_name_capture(state, NameKind::Rename)
}

/// `D` with the keys pane focused (M2 task 12): the same capture, prefilled
/// with the name plus `:copy`. Gated on the probed server version, since
/// `COPY` is Redis 6.2 and the floor is 6.0 (ADR-0007).
pub(super) fn begin_copy(state: State) -> (State, Vec<Command>) {
    if !state.copy_available() {
        let version = state
            .server_version()
            .map(|v| format!(" (this server is {v})"))
            .unwrap_or_default();
        return (
            state,
            vec![Command::Notify {
                text: format!("duplicate: needs Redis 6.2{version}"),
            }],
        );
    }
    begin_name_capture(state, NameKind::Copy)
}

/// Open the name capture for the Selected key.
///
/// A group row, or nothing selected, has no key to act on; a gone row has
/// nothing left; a name that is not valid UTF-8 cannot be edited in a text
/// capture without being rewritten, so it is refused (ADR-0017's rule for
/// List edit). Each says so rather than doing nothing silently.
fn begin_name_capture(mut state: State, kind: NameKind) -> (State, Vec<Command>) {
    let verb = kind.verb();
    let notify = |state: State, text: String| (state, vec![Command::Notify { text }]);
    let Some(index) = state.selected_key() else {
        return notify(state, format!("{verb}: select a key first"));
    };
    if state.keys.is_gone(index) {
        return notify(state, format!("{verb}: that key is gone"));
    }
    let Some(from) = state.keys.name(index).map(KeyName::from) else {
        return notify(state, format!("{verb}: select a key first"));
    };
    match RenameCapture::new(kind, index, from) {
        Some(capture) => {
            state.rename = Some(capture);
            (state, Vec::new())
        }
        None => notify(
            state,
            format!("{verb}: this key's name is binary (not valid UTF-8) and can't be typed"),
        ),
    }
}

/// Keys read while the rename capture is open: the keys-pane filter's
/// vocabulary — characters are text, `⌫` deletes, `Enter` stages, `Esc`
/// discards.
pub(super) fn rename_key(mut state: State, key: KeyPress) -> (State, Vec<Command>) {
    let Some(capture) = state.rename.as_mut() else {
        return (state, Vec::new());
    };
    match key.code {
        KeyCode::Esc => {
            state.rename = None;
            (state, Vec::new())
        }
        KeyCode::Enter => stage_name(state),
        KeyCode::Backspace => {
            capture.text.pop();
            (state, Vec::new())
        }
        KeyCode::Char(c) if !key.ctrl && !key.alt => {
            capture.text.push(c);
            (state, Vec::new())
        }
        _ => (state, Vec::new()),
    }
}

/// `Msg::Paste` into the capture: one line, newlines stripped.
pub(super) fn rename_paste(mut state: State, text: &str) -> (State, Vec<Command>) {
    if let Some(capture) = state.rename.as_mut() {
        capture
            .text
            .extend(text.chars().filter(|c| *c != '\n' && *c != '\r'));
    }
    (state, Vec::new())
}

/// `Enter` in the capture: refuse an empty or unchanged name (the reason is
/// already on screen), otherwise stage the rename or copy and ask for the
/// pre-check.
///
/// Nothing is sent that writes. The `EXISTS` is advice for the preview, and
/// is not sent at all when the rename can never run (cross-slot).
fn stage_name(mut state: State) -> (State, Vec<Command>) {
    let Some(capture) = state.rename.take() else {
        return (state, Vec::new());
    };
    if capture.problem().is_some() {
        state.rename = Some(capture);
        return (state, Vec::new());
    }
    // A rescan between `R` and `Enter` renumbers the Loaded set; the row may
    // now be a different key (the guard `key_deleted` applies at the other end).
    if state.keys.name(capture.index) != Some(capture.from.as_bytes()) {
        return (
            state,
            vec![Command::Notify {
                text: format!(
                    "{}: the key list changed — press {} again",
                    capture.kind.verb(),
                    capture.kind.key_label()
                ),
            }],
        );
    }
    let to = KeyName::from(capture.text.as_str());
    let apart = state.on_cluster()
        && crate::slot::key_slot(capture.from.as_bytes()) != crate::slot::key_slot(to.as_bytes());
    let (index, name) = (capture.index, capture.from);
    let (pending, check) = match capture.kind {
        NameKind::Rename => {
            let slots = if apart {
                SlotPath::CrossSlotRefused
            } else {
                SlotPath::SameSlot
            };
            let pending = PendingMutation::RenameKey {
                index,
                name,
                to: to.clone(),
                target: TargetCheck::Unchecked,
                slots,
            };
            (pending, !apart)
        }
        NameKind::Copy => {
            let slots = if apart {
                SlotPath::CrossSlotFallback
            } else {
                SlotPath::SameSlot
            };
            let pending = PendingMutation::CopyKey {
                index,
                name,
                to: to.clone(),
                target: TargetCheck::Unchecked,
                slots,
            };
            // A copy always runs, so it always gets the pre-check.
            (pending, true)
        }
    };
    state.confirm = Some(pending);
    let commands = if check {
        vec![Command::CheckTarget { key: to }]
    } else {
        Vec::new()
    };
    (state, commands)
}

/// `Msg::TargetChecked`: fold the `EXISTS` answer into the staged rename or
/// copy it was asked for. Dropped if nothing like it is staged any more —
/// after `Esc`, or after `y`, when `RENAMENX` / `COPY` is already the guard.
pub(super) fn target_checked(
    mut state: State,
    key: KeyName,
    exists: bool,
) -> (State, Vec<Command>) {
    if let Some((to, target)) = state.confirm.as_mut().and_then(PendingMutation::target_mut)
        && *to == key
    {
        *target = if exists {
            TargetCheck::Taken
        } else {
            TargetCheck::Free
        };
    }
    (state, Vec::new())
}

/// A rename completed: the old name is gone and the new one exists with the
/// same value and TTL.
///
/// The old row is marked gone, as a delete does. The new name enters the
/// Loaded set through [`scan_batch`], the same function a scanned page goes
/// through, so the cap is still enforced in exactly one place; if the set is
/// full the key is not listed and the notice says so. The Open key and the
/// selection follow, and the Open key re-arms through the one read path.
pub(super) fn key_renamed(
    mut state: State,
    index: Option<usize>,
    from: KeyName,
    to: KeyName,
    at_ms: u64,
) -> (State, Vec<Command>) {
    let from_index = index.filter(|&i| state.keys.name(i) == Some(from.as_bytes()));
    let was_selected = from_index.is_some() && state.selected_key() == from_index;
    if let Some(i) = from_index {
        state.keys.set_gone(i);
    }
    let (mut state, mut commands, new_index) = insert_loaded_key(state, &to);

    if let Some(ni) = new_index
        && was_selected
    {
        state.follow = Some(ni);
        settle_follow(&mut state);
        // No row and no job coming to make one: the filter or a collapsed
        // group hides the new key, so there is nothing to follow.
        if !state.rebuild_running() {
            state.follow = None;
        }
    }

    if state.open.as_ref().is_some_and(|o| o.name == from) {
        if let Some(open) = state.open.as_mut() {
            // Retargeted, not reopened: the old key's invalidation is a
            // nameless `Msg::Invalidated`, whose refetch reads `open.name`.
            // Left on the old name it would supersede the read below and
            // tombstone a key that was only renamed.
            open.write_landed();
            open.name = to.clone();
            open.index = new_index;
            open.deleted_at_ms = None;
            open.pending = None;
        }
        state.relocate_open_key();
        commands.extend(refetch(&mut state));
    }

    let tail = if new_index.is_none() {
        " (not listed: the key list is full)"
    } else {
        ""
    };
    state.notice = Some((format!("renamed {from} → {to}{tail}"), at_ms));
    (state, commands)
}

/// Put a key that just came into being on the server into the Loaded set,
/// through [`scan_batch`] like a scanned one, so the cap is enforced in that
/// one place. Returns its Loaded set index, or `None` when the set is full
/// and the key was not listed. A key with no row yet (a sorted or tree view
/// does not rebuild for a one-key batch) gets a rebuild started, which keeps
/// the selection on its key. Shared by rename and copy.
fn insert_loaded_key(state: State, name: &KeyName) -> (State, Vec<Command>, Option<usize>) {
    let before = state.keys.len();
    let (mut state, commands) = scan_batch(state, vec![name.as_bytes().to_vec()]);
    let new_index = (state.keys.len() > before).then_some(before);
    if let Some(ni) = new_index
        && state.row_of(ni).is_none()
        && !state.rebuild_running()
    {
        state.rebuild_list_async();
    }
    (state, commands, new_index)
}

/// A copy completed: the new key exists with the source's value and TTL.
///
/// Only the new name is inserted. The selection and the Open key stay on the
/// source (decision 4): a duplicate is a side effect, not a move, so nothing
/// follows, nothing is marked gone, and the Open key is not retargeted.
pub(super) fn key_copied(
    state: State,
    from: KeyName,
    to: KeyName,
    at_ms: u64,
) -> (State, Vec<Command>) {
    let (mut state, commands, new_index) = insert_loaded_key(state, &to);
    let tail = if new_index.is_none() {
        " (not listed: the key list is full)"
    } else {
        ""
    };
    state.notice = Some((format!("copied {from} → {to}{tail}"), at_ms));
    (state, commands)
}

/// A guard refused the rename or copy and nothing was written. Always raised as an
/// error naming `RENAMENX old new` / `COPY src dst` — unlike the value edits it does not
/// depend on the key being open, because the Selected key is what `R` renames.
pub(super) fn rename_not_written(
    mut state: State,
    mutation: &Mutation,
    index: Option<usize>,
    why: NotWritten,
    at_ms: u64,
) -> (State, Vec<Command>) {
    if why == NotWritten::KeyGone
        && let Mutation::RenameKey { key, .. } | Mutation::CopyKey { key, .. } = mutation
    {
        if let Some(i) = index.filter(|&i| state.keys.name(i) == Some(key.as_bytes())) {
            state.keys.set_gone(i);
        }
        if let Some(open) = state.open.as_mut().filter(|o| o.name == *key) {
            open.deleted_at_ms = Some(at_ms);
            open.pending = None;
        }
    }
    state.error = Some((
        format!(
            "{}: {} — nothing written",
            settled_label(&state, mutation),
            why.reason()
        ),
        at_ms,
    ));
    (state, Vec::new())
}

/// Move the selection onto the followed row once it has one.
pub(super) fn settle_follow(state: &mut State) {
    if let Some(index) = state.follow
        && let Some(row) = state.row_of(index)
    {
        state.view.selected = row;
        state.view = state.view.scrolled_to_selection(state.visible_rows());
        state.follow = None;
        state.relocate_open_key();
    }
}

/// After a rebuild job swapped in covering `covered` keys: resolve a pending
/// follow. A followed key the swap covers but has no row for is hidden by the
/// filter or a collapsed group, so the follow is dropped; one it does not
/// cover needs another job.
pub(super) fn follow_after_swap(state: &mut State, covered: usize) {
    settle_follow(state);
    if let Some(index) = state.follow {
        if covered > index {
            state.follow = None;
        } else if !state.rebuild_running() {
            state.rebuild_list_async();
            settle_follow(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::value::{StringValue, Value};
    use crate::state::{Link, LoadedSet, OpenKey, Topology, Tracking};

    fn press(state: State, c: char) -> (State, Vec<Command>) {
        update(state, Msg::Key(KeyPress::plain(KeyCode::Char(c))))
    }

    fn named(state: State, code: KeyCode) -> (State, Vec<Command>) {
        update(state, Msg::Key(KeyPress::plain(code)))
    }

    fn type_text(mut state: State, text: &str) -> State {
        for c in text.chars() {
            state = press(state, c).0;
        }
        state
    }

    fn with_keys(names: &[&str]) -> State {
        let (s, _) = update(
            State {
                cols: 130,
                rows: 30,
                link: Link::Up {
                    version: "8.4.0".into(),
                    tracking: Tracking::Armed,
                },
                ..State::default()
            },
            Msg::ScanBatch {
                keys: names.iter().map(|n| n.as_bytes().to_vec()).collect(),
            },
        );
        s
    }

    /// `R`, retype the whole name, `Enter`: a staged rename of key 0.
    fn staged(state: State, new_name: &str) -> (State, Vec<Command>) {
        let (mut s, _) = press(state, 'R');
        let from_len = s
            .rename
            .as_ref()
            .expect("capture open")
            .text
            .chars()
            .count();
        for _ in 0..from_len {
            s = named(s, KeyCode::Backspace).0;
        }
        let s = type_text(s, new_name);
        named(s, KeyCode::Enter)
    }

    fn cluster(mut state: State) -> State {
        state.connection.topology = Some(Topology {
            primaries: 3,
            nodes: 6,
        });
        state
    }

    fn settle(state: State, mutation: Mutation, outcome: MutationOutcome) -> (State, Vec<Command>) {
        update(
            state,
            Msg::MutationSettled {
                mutation,
                index: Some(0),
                result: Ok(outcome),
                at_ms: 5_000,
            },
        )
    }

    fn rename_mutation(from: &str, to: &str) -> Mutation {
        Mutation::RenameKey {
            key: from.into(),
            to: to.into(),
        }
    }

    // ---- the capture ----------------------------------------------------

    #[test]
    fn r_opens_a_capture_prefilled_with_the_current_name() {
        let (s, cmds) = press(with_keys(&["user:1"]), 'R');
        assert!(cmds.is_empty());
        let capture = s.rename.as_ref().expect("capture open");
        assert_eq!(capture.text, "user:1");
        assert_eq!(capture.index, 0);
        assert_eq!(mode(&s), Mode::Renaming);
    }

    #[test]
    fn typing_and_backspace_edit_the_name_at_the_end() {
        let (s, _) = press(with_keys(&["ab"]), 'R');
        let s = named(s, KeyCode::Backspace).0;
        let s = type_text(s, "xyz");
        assert_eq!(s.rename.as_ref().unwrap().text, "axyz");
    }

    #[test]
    fn esc_discards_the_capture_and_stages_nothing() {
        let (s, _) = press(with_keys(&["k"]), 'R');
        let (s, cmds) = named(s, KeyCode::Esc);
        assert!(s.rename.is_none());
        assert!(s.confirm.is_none());
        assert!(cmds.is_empty());
        assert_eq!(mode(&s), Mode::Normal);
    }

    #[test]
    fn an_unchanged_name_is_refused_inline() {
        let (s, _) = press(with_keys(&["k"]), 'R');
        assert_eq!(
            s.rename.as_ref().unwrap().problem(),
            Some(crate::state::RenameProblem::Unchanged)
        );
        let (s, cmds) = named(s, KeyCode::Enter);
        assert!(s.confirm.is_none(), "nothing staged");
        assert!(s.rename.is_some(), "the capture stays open to be fixed");
        assert!(cmds.is_empty());
    }

    #[test]
    fn an_empty_name_is_refused_inline() {
        let (s, _) = press(with_keys(&["k"]), 'R');
        let s = named(s, KeyCode::Backspace).0;
        assert_eq!(
            s.rename.as_ref().unwrap().problem(),
            Some(crate::state::RenameProblem::Empty)
        );
        let (s, cmds) = named(s, KeyCode::Enter);
        assert!(s.confirm.is_none());
        assert!(s.rename.is_some());
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_binary_name_is_refused_with_a_notice() {
        let (s, _) = update(
            State::default(),
            Msg::ScanBatch {
                keys: vec![vec![0xff, 0xfe, b'k']],
            },
        );
        let (s, cmds) = press(s, 'R');
        assert!(s.rename.is_none());
        assert!(matches!(&cmds[..], [Command::Notify { text }] if text.contains("binary")));
    }

    #[test]
    fn r_on_a_group_row_or_nothing_says_so_and_stages_nothing() {
        let (s, cmds) = press(State::default(), 'R');
        assert!(s.rename.is_none());
        assert!(matches!(&cmds[..], [Command::Notify { text }] if text.contains("select a key")));

        let mut s = with_keys(&["user:1:a", "user:1:b"]);
        s.tree_mode = true;
        s.rebuild_list();
        s.view.selected = 0;
        assert!(
            matches!(s.tree.row(0), Some(crate::state::tree::Row::Group { .. })),
            "row 0 is a group"
        );
        let (s, cmds) = press(s, 'R');
        assert!(s.rename.is_none());
        assert!(matches!(&cmds[..], [Command::Notify { .. }]));
    }

    #[test]
    fn r_on_a_gone_row_is_refused() {
        let mut s = with_keys(&["k"]);
        s.keys.set_gone(0);
        let (s, cmds) = press(s, 'R');
        assert!(s.rename.is_none());
        assert!(matches!(&cmds[..], [Command::Notify { text }] if text.contains("gone")));
    }

    #[test]
    fn r_in_the_value_pane_does_nothing() {
        let mut s = with_keys(&["k"]);
        s.focus = Pane::Value;
        s.open = Some(OpenKey::new(
            Some(0),
            "k".into(),
            Value::Str(StringValue::new("v", 1)),
            -1,
            1,
            0,
        ));
        let (s, cmds) = press(s, 'R');
        assert!(s.rename.is_none());
        assert!(s.confirm.is_none());
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_paste_goes_into_the_capture_without_newlines() {
        let (s, _) = press(with_keys(&["k"]), 'R');
        let (s, _) = update(s, Msg::Paste("a\nb".into()));
        assert_eq!(s.rename.as_ref().unwrap().text, "kab");
    }

    #[test]
    fn f1_opens_help_over_the_capture_and_the_capture_survives() {
        let (s, _) = press(with_keys(&["k"]), 'R');
        let (s, _) = named(s, KeyCode::F(1));
        assert!(s.help.is_some());
        assert_eq!(
            crate::help::context(&s),
            crate::help::HelpContext::Rename(NameKind::Rename),
            "help describes the capture beneath it"
        );
        let (s, _) = named(s, KeyCode::Esc);
        assert!(s.rename.is_some());
    }

    // ---- staging, preview and the pre-check -----------------------------

    #[test]
    fn enter_stages_renamenx_and_asks_for_the_precheck() {
        let (s, cmds) = staged(with_keys(&["old"]), "new");
        assert!(s.rename.is_none());
        let pending = s.confirm.as_ref().expect("staged");
        assert_eq!(pending.command_text(), "RENAMENX old new");
        assert_eq!(cmds, vec![Command::CheckTarget { key: "new".into() }]);
        match pending {
            PendingMutation::RenameKey {
                index,
                target,
                slots,
                ..
            } => {
                assert_eq!(*index, 0);
                assert_eq!(*target, TargetCheck::Unchecked);
                assert_eq!(*slots, SlotPath::SameSlot);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn y_executes_renamenx_with_the_row_it_came_from() {
        let (s, _) = staged(with_keys(&["old"]), "new");
        let (s, cmds) = press(s, 'y');
        assert!(s.confirm.is_none());
        assert_eq!(
            cmds,
            vec![Command::Execute {
                mutation: rename_mutation("old", "new"),
                index: Some(0),
            }]
        );
    }

    #[test]
    fn read_only_mode_refuses_at_confirm_after_the_preview_is_composed() {
        let mut s = with_keys(&["old"]);
        s.read_only = Some(crate::state::ReadOnlyReason::Environment);
        let (s, _) = staged(s, "new");
        assert!(s.confirm.is_some(), "the preview is composed anyway");
        let (s, cmds) = press(s, 'y');
        assert!(s.confirm.is_none());
        assert!(
            !cmds.iter().any(|c| matches!(c, Command::Execute { .. })),
            "nothing is sent"
        );
        assert!(matches!(&cmds[..], [Command::Notify { text }] if text.contains("read-only")));
    }

    #[test]
    fn the_precheck_answer_lands_on_the_staged_dialog() {
        let (s, _) = staged(with_keys(&["old"]), "new");
        let (s, _) = update(
            s,
            Msg::TargetChecked {
                key: "new".into(),
                exists: true,
            },
        );
        assert!(matches!(
            s.confirm,
            Some(PendingMutation::RenameKey {
                target: TargetCheck::Taken,
                ..
            })
        ));
        let (s, _) = update(
            s,
            Msg::TargetChecked {
                key: "new".into(),
                exists: false,
            },
        );
        assert!(matches!(
            s.confirm,
            Some(PendingMutation::RenameKey {
                target: TargetCheck::Free,
                ..
            })
        ));
    }

    #[test]
    fn a_taken_target_does_not_block_y() {
        let (s, _) = staged(with_keys(&["old"]), "new");
        let (s, _) = update(
            s,
            Msg::TargetChecked {
                key: "new".into(),
                exists: true,
            },
        );
        let (_, cmds) = press(s, 'y');
        assert!(
            cmds.iter().any(|c| matches!(c, Command::Execute { .. })),
            "the atomic RENAMENX is the guard, the pre-check is advice"
        );
    }

    #[test]
    fn a_precheck_arriving_after_y_is_dropped() {
        let (s, _) = staged(with_keys(&["old"]), "new");
        let (s, _) = press(s, 'y');
        let before = s.clone();
        let (after, cmds) = update(
            s,
            Msg::TargetChecked {
                key: "new".into(),
                exists: true,
            },
        );
        assert_eq!(after, before);
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_precheck_arriving_after_esc_is_dropped() {
        let (s, _) = staged(with_keys(&["old"]), "new");
        let (s, _) = named(s, KeyCode::Esc);
        let before = s.clone();
        let (after, _) = update(
            s,
            Msg::TargetChecked {
                key: "new".into(),
                exists: true,
            },
        );
        assert_eq!(after, before);
    }

    #[test]
    fn a_precheck_for_another_name_is_ignored() {
        let (s, _) = staged(with_keys(&["old"]), "new");
        let (s, _) = update(
            s,
            Msg::TargetChecked {
                key: "other".into(),
                exists: true,
            },
        );
        assert!(matches!(
            s.confirm,
            Some(PendingMutation::RenameKey {
                target: TargetCheck::Unchecked,
                ..
            })
        ));
    }

    // ---- Cluster ---------------------------------------------------------

    #[test]
    fn across_slots_on_a_cluster_y_does_nothing_and_nothing_is_sent() {
        let s = cluster(with_keys(&["a"]));
        let (s, cmds) = staged(s, "b");
        assert!(
            cmds.is_empty(),
            "no pre-check for a rename that can never run"
        );
        assert!(matches!(
            s.confirm,
            Some(PendingMutation::RenameKey {
                slots: SlotPath::CrossSlotRefused,
                ..
            })
        ));
        let (s, cmds) = press(s, 'y');
        assert!(s.confirm.is_some(), "the dialog stays");
        assert!(cmds.is_empty());
        let (s, _) = named(s, KeyCode::Esc);
        assert!(s.confirm.is_none(), "only Esc leaves");
    }

    #[test]
    fn across_slots_is_refused_under_read_only_too_without_closing_on_the_wrong_reason() {
        let mut s = cluster(with_keys(&["a"]));
        s.read_only = Some(crate::state::ReadOnlyReason::Environment);
        let (s, _) = staged(s, "b");
        let (s, cmds) = press(s, 'y');
        assert!(s.confirm.is_some());
        assert!(cmds.is_empty());
    }

    #[test]
    fn the_same_slot_through_a_hash_tag_runs_on_a_cluster() {
        let s = cluster(with_keys(&["{x}a"]));
        let (s, cmds) = staged(s, "{x}b");
        assert!(matches!(
            s.confirm,
            Some(PendingMutation::RenameKey {
                slots: SlotPath::SameSlot,
                ..
            })
        ));
        assert_eq!(cmds, vec![Command::CheckTarget { key: "{x}b".into() }]);
        let (_, cmds) = press(s, 'y');
        assert!(cmds.iter().any(|c| matches!(c, Command::Execute { .. })));
    }

    #[test]
    fn off_a_cluster_slots_never_matter() {
        let (s, _) = staged(with_keys(&["a"]), "b");
        assert!(matches!(
            s.confirm,
            Some(PendingMutation::RenameKey {
                slots: SlotPath::SameSlot,
                ..
            })
        ));
    }

    // ---- outcomes --------------------------------------------------------

    #[test]
    fn a_completed_rename_marks_the_old_row_gone_and_lists_the_new_key() {
        let s = with_keys(&["a", "b"]);
        let (s, _) = settle(s, rename_mutation("a", "z"), MutationOutcome::Done);
        assert!(s.keys.is_gone(0));
        assert_eq!(s.keys.len(), 3);
        assert_eq!(s.keys.name(2), Some(&b"z"[..]));
        assert_eq!(s.row_of(2), Some(2), "the new key has a row");
        let (text, _) = s.notice.as_ref().unwrap();
        assert!(text.contains("renamed a") && text.contains("z"), "{text}");
    }

    #[test]
    fn the_selection_moves_to_the_new_row() {
        let s = with_keys(&["a", "b"]);
        assert_eq!(s.selected_key(), Some(0));
        let (s, _) = settle(s, rename_mutation("a", "z"), MutationOutcome::Done);
        assert_eq!(s.selected_key(), Some(2));
        assert!(s.follow.is_none());
    }

    #[test]
    fn a_selection_the_reader_moved_is_left_alone() {
        let mut s = with_keys(&["a", "b"]);
        s.view.selected = 1;
        let (s, _) = settle(s, rename_mutation("a", "z"), MutationOutcome::Done);
        assert_eq!(s.selected_key(), Some(1));
    }

    #[test]
    fn the_open_key_follows_and_rereads_through_the_normal_path() {
        let mut s = with_keys(&["a", "b"]);
        let mut open = OpenKey::new(
            Some(0),
            "a".into(),
            Value::Str(StringValue::new("v", 1)),
            -1,
            1,
            0,
        );
        open.row = Some(0);
        s.open = Some(open);
        let before_token = s.read_token;
        let (s, cmds) = settle(s, rename_mutation("a", "z"), MutationOutcome::Done);
        let open = s.open.as_ref().unwrap();
        assert_eq!(open.name, KeyName::from("z"));
        assert_eq!(open.index, Some(2));
        assert_eq!(open.row, Some(2));
        assert!(open.deleted_at_ms.is_none(), "never tombstoned");
        let read = cmds
            .iter()
            .find_map(|c| match c {
                Command::ReadKey {
                    key,
                    index,
                    token,
                    arm,
                } => Some((key.clone(), *index, *token, *arm)),
                _ => None,
            })
            .expect("a read is issued");
        assert_eq!(read.0, KeyName::from("z"));
        assert_eq!(read.1, Some(2));
        assert_ne!(read.2, before_token);
        assert!(read.3, "the read re-arms tracking");
        assert_eq!(s.open_pending.as_ref().unwrap().name, KeyName::from("z"));
    }

    #[test]
    fn an_invalidation_after_the_rename_refetches_the_new_name_not_the_old() {
        let mut s = with_keys(&["a"]);
        s.open = Some(OpenKey::new(
            Some(0),
            "a".into(),
            Value::Str(StringValue::new("v", 1)),
            -1,
            1,
            0,
        ));
        let (s, _) = settle(s, rename_mutation("a", "z"), MutationOutcome::Done);
        let (_, cmds) = update(s, Msg::Invalidated);
        assert!(
            matches!(&cmds[..], [Command::ReadKey { key, .. }] if key.as_bytes() == b"z"),
            "{cmds:?}"
        );
    }

    #[test]
    fn a_rename_of_another_key_leaves_the_open_key_alone() {
        let mut s = with_keys(&["a", "b"]);
        s.open = Some(OpenKey::new(
            Some(1),
            "b".into(),
            Value::Str(StringValue::new("v", 1)),
            -1,
            1,
            0,
        ));
        let (s, cmds) = settle(s, rename_mutation("a", "z"), MutationOutcome::Done);
        assert_eq!(s.open.as_ref().unwrap().name, KeyName::from("b"));
        assert!(!cmds.iter().any(|c| matches!(c, Command::ReadKey { .. })));
    }

    #[test]
    fn the_new_key_asks_for_its_metadata() {
        let (_, cmds) = settle(
            with_keys(&["a"]),
            rename_mutation("a", "z"),
            MutationOutcome::Done,
        );
        assert!(
            cmds.iter().any(
                |c| matches!(c, Command::FetchMetadata { indices, .. } if indices.contains(&1))
            ),
            "{cmds:?}"
        );
    }

    #[test]
    fn the_cap_still_holds_when_the_loaded_set_is_full() {
        let mut s = with_keys(&[]);
        s.keys = LoadedSet::with_cap(2);
        let (mut s, _) = update(
            s,
            Msg::ScanBatch {
                keys: vec![b"a".to_vec(), b"b".to_vec()],
            },
        );
        s.open = Some(OpenKey::new(
            Some(0),
            "a".into(),
            Value::Str(StringValue::new("v", 1)),
            -1,
            1,
            0,
        ));
        let (s, cmds) = settle(s, rename_mutation("a", "z"), MutationOutcome::Done);
        assert_eq!(s.keys.len(), 2, "the cap is enforced in scan_batch alone");
        assert!(s.keys.is_capped());
        assert!(s.keys.is_gone(0));
        let (text, _) = s.notice.as_ref().unwrap();
        assert!(text.contains("not listed"), "{text}");
        // The Open key still follows by name, off the list.
        let open = s.open.as_ref().unwrap();
        assert_eq!(open.name, KeyName::from("z"));
        assert_eq!(open.index, None);
        assert!(
            cmds.iter()
                .any(|c| matches!(c, Command::ReadKey { key, .. } if key.as_bytes() == b"z"))
        );
    }

    #[test]
    fn a_taken_target_is_reported_naming_the_command_even_with_nothing_open() {
        let s = with_keys(&["a", "b"]);
        let (s, cmds) = settle(
            s,
            rename_mutation("a", "b"),
            MutationOutcome::NotWritten(NotWritten::TargetExists),
        );
        let (text, _) = s.error.as_ref().expect("an error, not silence");
        assert!(text.contains("RENAMENX a b"), "{text}");
        assert!(text.contains("already exists"), "{text}");
        assert!(text.contains("nothing written"), "{text}");
        assert!(!s.keys.is_gone(0), "the source is untouched");
        assert_eq!(s.keys.len(), 2, "nothing was added");
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_gone_source_marks_the_row_and_creates_nothing() {
        let s = with_keys(&["a"]);
        let (s, _) = settle(
            s,
            rename_mutation("a", "z"),
            MutationOutcome::NotWritten(NotWritten::KeyGone),
        );
        assert!(s.keys.is_gone(0));
        assert_eq!(s.keys.len(), 1, "no new key");
        let (text, _) = s.error.as_ref().unwrap();
        assert!(
            text.contains("RENAMENX a z") && text.contains("no longer exists"),
            "{text}"
        );
    }

    #[test]
    fn a_server_error_names_the_failing_command() {
        let s = with_keys(&["a"]);
        let (s, _) = update(
            s,
            Msg::MutationSettled {
                mutation: rename_mutation("a", "z"),
                index: Some(0),
                result: Err("READONLY You can't write against a read only replica".into()),
                at_ms: 1,
            },
        );
        let (text, _) = s.error.as_ref().unwrap();
        assert!(text.starts_with("RENAMENX a z: READONLY"), "{text}");
    }

    /// A tree-mode keyspace large enough that the rebuild is a sliced job:
    /// the new row does not exist when the reply lands, and the selection
    /// follows it once the job swaps in.
    #[test]
    fn the_selection_follows_through_a_sliced_rebuild_in_tree_mode() {
        let names: Vec<String> = (0..40).map(|i| format!("k{i:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut s = State {
            tree_mode: true,
            rebuild_slice: Some(4),
            ..State::default()
        };
        s.cols = 130;
        s.rows = 30;
        let (mut s, _) = update(
            s,
            Msg::ScanBatch {
                keys: refs.iter().map(|n| n.as_bytes().to_vec()).collect(),
            },
        );
        s.run_rebuild_to_completion();
        s.view.selected = s.row_of(3).expect("row");
        let (mut s, _) = settle_at(s, 3, "k03", "zz-new");
        // Drive the shell's steps.
        for _ in 0..500 {
            if !s.rebuild_running() {
                break;
            }
            s = update(s, Msg::RebuildStep).0;
        }
        let new_index = s.keys.len() - 1;
        assert_eq!(s.keys.name(new_index), Some(&b"zz-new"[..]));
        assert_eq!(s.selected_key(), Some(new_index), "follow landed");
        assert!(s.follow.is_none());
    }

    fn settle_at(state: State, index: usize, from: &str, to: &str) -> (State, Vec<Command>) {
        update(
            state,
            Msg::MutationSettled {
                mutation: rename_mutation(from, to),
                index: Some(index),
                result: Ok(MutationOutcome::Done),
                at_ms: 1,
            },
        )
    }

    #[test]
    fn a_new_name_hidden_by_the_filter_drops_the_follow_without_moving() {
        let mut s = with_keys(&["a1", "a2"]);
        s.list.filter = "a".into();
        s.rebuild_list();
        let (s, _) = settle(s, rename_mutation("a1", "zzz"), MutationOutcome::Done);
        assert!(s.follow.is_none());
        assert_eq!(s.keys.len(), 3);
        assert!(s.row_of(2).is_none(), "filtered out, not shown");
    }

    #[test]
    fn a_rescan_clears_a_pending_follow() {
        let mut s = with_keys(&["a"]);
        s.follow = Some(0);
        let (s, _) = update(s, Msg::ScanStarted { estimated_total: 0 });
        assert!(s.follow.is_none());
    }

    #[test]
    fn a_keypress_takes_a_pending_follow_back() {
        let mut s = with_keys(&["a", "b"]);
        s.follow = Some(1);
        let (s, _) = press(s, 'j');
        assert!(s.follow.is_none());
    }

    #[test]
    fn the_mouse_is_inert_while_the_capture_is_open() {
        let (s, _) = press(with_keys(&["a", "b"]), 'R');
        let before = s.clone();
        let (after, _) = update(
            s,
            Msg::Mouse(crate::msg::MouseAction::ScrollDown { col: 2, row: 3 }),
        );
        assert_eq!(after, before);
    }

    #[test]
    fn a_rescan_between_r_and_enter_stages_nothing() {
        let (s, _) = press(with_keys(&["a"]), 'R');
        let (s, _) = update(s, Msg::ScanStarted { estimated_total: 0 });
        let (s, _) = update(
            s,
            Msg::ScanBatch {
                keys: vec![b"other".to_vec()],
            },
        );
        let (s, cmds) = type_text_enter(s);
        assert!(s.confirm.is_none());
        assert!(matches!(&cmds[..], [Command::Notify { .. }]));
    }

    fn type_text_enter(state: State) -> (State, Vec<Command>) {
        let s = type_text(state, "x");
        named(s, KeyCode::Enter)
    }

    // ---- duplicate (M2 task 12) ------------------------------------------

    fn on_version(mut state: State, version: &str) -> State {
        state.link = Link::Up {
            version: version.into(),
            tracking: Tracking::Armed,
        };
        state
    }

    fn copy_mutation(from: &str, to: &str) -> Mutation {
        Mutation::CopyKey {
            key: from.into(),
            to: to.into(),
        }
    }

    /// `D`, retype the whole name, `Enter`: a staged copy of key 0.
    fn staged_copy(state: State, new_name: &str) -> (State, Vec<Command>) {
        let (mut s, _) = press(state, 'D');
        let n = s
            .rename
            .as_ref()
            .expect("capture open")
            .text
            .chars()
            .count();
        for _ in 0..n {
            s = named(s, KeyCode::Backspace).0;
        }
        let s = type_text(s, new_name);
        named(s, KeyCode::Enter)
    }

    #[test]
    fn d_opens_a_capture_prefilled_with_the_name_and_a_copy_suffix() {
        let (s, cmds) = press(with_keys(&["user:1"]), 'D');
        assert!(cmds.is_empty());
        let capture = s.rename.as_ref().expect("capture open");
        assert_eq!(capture.text, "user:1:copy");
        assert_eq!(capture.kind, NameKind::Copy);
        assert_eq!(mode(&s), Mode::Renaming);
        assert_eq!(
            crate::help::context(&s),
            crate::help::HelpContext::Rename(NameKind::Copy)
        );
    }

    #[test]
    fn the_copy_prefill_is_not_flagged_as_unchanged() {
        let (s, _) = press(with_keys(&["k"]), 'D');
        assert_eq!(s.rename.as_ref().unwrap().problem(), None);
    }

    #[test]
    fn deleting_the_suffix_leaves_an_unchanged_name_refused_inline() {
        let (mut s, _) = press(with_keys(&["k"]), 'D');
        for _ in 0..COPY_SUFFIX_LEN {
            s = named(s, KeyCode::Backspace).0;
        }
        let capture = s.rename.as_ref().unwrap();
        assert_eq!(
            capture.problem(),
            Some(crate::state::RenameProblem::Unchanged)
        );
        assert_eq!(
            capture.problem().unwrap().reason(capture.kind),
            "same as the source name"
        );
        let (s, cmds) = named(s, KeyCode::Enter);
        assert!(s.confirm.is_none() && s.rename.is_some() && cmds.is_empty());
    }

    const COPY_SUFFIX_LEN: usize = crate::state::rename::COPY_SUFFIX.len();

    #[test]
    fn d_on_nothing_or_a_binary_name_says_so_and_stages_nothing() {
        let (s, cmds) = press(State::default(), 'D');
        assert!(s.rename.is_none());
        assert!(matches!(&cmds[..], [Command::Notify { text }] if text.starts_with("duplicate:")));
        let (s, _) = update(
            State::default(),
            Msg::ScanBatch {
                keys: vec![vec![0xff, 0xfe]],
            },
        );
        let (s, cmds) = press(s, 'D');
        assert!(s.rename.is_none());
        assert!(matches!(&cmds[..], [Command::Notify { text }] if text.contains("binary")));
    }

    #[test]
    fn d_in_the_value_pane_does_nothing() {
        let mut s = with_keys(&["k"]);
        s.focus = Pane::Value;
        let (s, cmds) = press(s, 'D');
        assert!(s.rename.is_none() && s.confirm.is_none() && cmds.is_empty());
    }

    #[test]
    fn enter_stages_copy_and_asks_for_the_precheck() {
        let (s, cmds) = staged_copy(with_keys(&["src"]), "dst");
        assert!(s.rename.is_none());
        let pending = s.confirm.as_ref().expect("staged");
        assert_eq!(pending.command_text(), "COPY src dst");
        assert_eq!(pending.command_lines(), vec!["COPY src dst".to_string()]);
        assert_eq!(cmds, vec![Command::CheckTarget { key: "dst".into() }]);
        assert!(matches!(
            pending,
            PendingMutation::CopyKey {
                index: 0,
                target: TargetCheck::Unchecked,
                slots: SlotPath::SameSlot,
                ..
            }
        ));
    }

    #[test]
    fn the_precheck_answer_lands_on_a_staged_copy_and_late_ones_are_dropped() {
        let (s, _) = staged_copy(with_keys(&["src"]), "dst");
        let (s, _) = update(
            s,
            Msg::TargetChecked {
                key: "dst".into(),
                exists: true,
            },
        );
        assert!(matches!(
            s.confirm,
            Some(PendingMutation::CopyKey {
                target: TargetCheck::Taken,
                ..
            })
        ));
        let (s, _) = update(
            s,
            Msg::TargetChecked {
                key: "other".into(),
                exists: false,
            },
        );
        assert!(matches!(
            s.confirm,
            Some(PendingMutation::CopyKey {
                target: TargetCheck::Taken,
                ..
            })
        ));
        // `y` is never blocked by Taken: COPY is the atomic guard.
        let (_, cmds) = press(s, 'y');
        assert!(matches!(
            &cmds[..],
            [Command::Execute {
                mutation: Mutation::CopyKey { .. },
                index: Some(0)
            }]
        ));
        let (s, _) = staged_copy(with_keys(&["src"]), "dst");
        let (s, _) = named(s, KeyCode::Esc);
        let (s, _) = update(
            s,
            Msg::TargetChecked {
                key: "dst".into(),
                exists: true,
            },
        );
        assert!(s.confirm.is_none());
    }

    #[test]
    fn copy_across_slots_falls_back_and_y_still_runs() {
        let s = cluster(with_keys(&["a"]));
        let (s, cmds) = staged_copy(s, "b");
        assert_eq!(cmds, vec![Command::CheckTarget { key: "b".into() }]);
        let pending = s.confirm.as_ref().unwrap();
        assert!(matches!(
            pending,
            PendingMutation::CopyKey {
                slots: SlotPath::CrossSlotFallback,
                ..
            }
        ));
        assert!(!pending.blocks_confirm());
        assert_eq!(
            pending.command_lines(),
            vec![
                "DUMP a".to_string(),
                "PTTL a".to_string(),
                "RESTORE b <ttl> <payload>".to_string()
            ]
        );
        let (_, cmds) = press(s, 'y');
        assert!(cmds.iter().any(|c| matches!(c, Command::Execute { .. })));
    }

    #[test]
    fn copy_within_a_hash_tag_slot_on_a_cluster_is_a_plain_copy() {
        let (s, _) = staged_copy(cluster(with_keys(&["{x}a"])), "{x}b");
        assert!(matches!(
            s.confirm,
            Some(PendingMutation::CopyKey {
                slots: SlotPath::SameSlot,
                ..
            })
        ));
    }

    #[test]
    fn read_only_refuses_a_copy_at_confirm_after_the_preview() {
        let mut s = with_keys(&["src"]);
        s.read_only = Some(crate::state::ReadOnlyReason::Environment);
        let (s, _) = staged_copy(s, "dst");
        assert!(s.confirm.is_some(), "the preview still opens");
        let (s, cmds) = press(s, 'y');
        assert!(s.confirm.is_none());
        assert!(matches!(&cmds[..], [Command::Notify { text }] if text.contains("read-only")));
    }

    #[test]
    fn below_redis_6_2_d_is_refused_with_the_reason_and_stages_nothing() {
        for v in ["6.0.20", "6.1.9"] {
            let s = on_version(with_keys(&["k"]), v);
            assert!(!s.copy_available());
            let (s, cmds) = press(s, 'D');
            assert!(s.rename.is_none(), "{v}");
            assert!(
                matches!(&cmds[..], [Command::Notify { text }] if text.contains("needs Redis 6.2") && text.contains(v)),
                "{cmds:?}"
            );
        }
    }

    #[test]
    fn from_6_2_and_with_an_unknown_version_d_is_available() {
        for v in ["6.2.0", "7.0.0", "8.4.0"] {
            assert!(on_version(State::default(), v).copy_available(), "{v}");
        }
        assert!(State::default().copy_available(), "unknown is allowed");
        let mut s = with_keys(&["k"]);
        s.link = Link::Connecting;
        assert!(s.copy_available());
    }

    #[test]
    fn the_help_row_is_dimmed_with_the_version_reason_below_6_2() {
        let row = |s: &State| {
            crate::help::here(s, crate::help::context(s))
                .into_iter()
                .find(|r| r.label == "duplicate key (COPY)")
                .expect("the keys pane lists duplicate")
        };
        let ok = with_keys(&["k"]);
        assert_eq!(row(&ok).keys, "D");
        assert!(row(&ok).refused.is_none());
        let old = on_version(with_keys(&["k"]), "6.0.9");
        assert_eq!(
            row(&old).refused.map(|r| r.text()).as_deref(),
            Some("needs Redis 6.2")
        );
        // The version gate outranks Read-only dimming.
        let mut both = old;
        both.read_only = Some(crate::state::ReadOnlyReason::Environment);
        assert_eq!(
            row(&both).refused.map(|r| r.text()).as_deref(),
            Some("needs Redis 6.2")
        );
        let mut ro = with_keys(&["k"]);
        ro.read_only = Some(crate::state::ReadOnlyReason::Environment);
        assert_eq!(
            row(&ro).refused.map(|r| r.text()).as_deref(),
            Some("read-only (environment) · preview only")
        );
    }

    #[test]
    fn a_completed_copy_lists_the_new_key_and_leaves_everything_else_alone() {
        let mut s = with_keys(&["a", "b"]);
        s.view.selected = 1;
        let mut open = OpenKey::new(
            Some(1),
            "b".into(),
            Value::Str(StringValue::new("v", 1)),
            -1,
            1,
            0,
        );
        open.row = Some(1);
        s.open = Some(open);
        let token = s.read_token;
        let (s, cmds) = update(
            s,
            Msg::MutationSettled {
                mutation: copy_mutation("b", "z"),
                index: Some(1),
                result: Ok(MutationOutcome::Done),
                at_ms: 5_000,
            },
        );
        assert_eq!(s.keys.len(), 3, "the new key entered through scan_batch");
        assert_eq!(s.keys.name(2), Some(&b"z"[..]));
        assert!(!s.keys.is_gone(1), "the source is not marked gone");
        assert_eq!(s.view.selected, 1, "the selection stays on the source");
        assert_eq!(s.selected_key(), Some(1));
        assert!(s.follow.is_none());
        let open = s.open.as_ref().unwrap();
        assert_eq!(open.name, KeyName::from("b"), "the Open key stays put");
        assert_eq!(s.read_token, token, "no re-read of the Open key");
        assert!(
            !cmds.iter().any(|c| matches!(c, Command::ReadKey { .. })),
            "{cmds:?}"
        );
        assert!(s.notice.as_ref().unwrap().0.contains("copied b"));
    }

    #[test]
    fn a_copy_in_a_sorted_view_keeps_the_selection_on_the_source() {
        let mut s = with_keys(&["b", "c", "d"]);
        s.list.filter = String::new();
        s.view.selected = 2;
        assert_eq!(s.selected_key(), Some(2));
        let (s, _) = settle(s, copy_mutation("d", "a"), MutationOutcome::Done);
        let selected = s.selected_key().expect("still selected");
        assert_eq!(s.keys.name(selected), Some(&b"d"[..]));
    }

    #[test]
    fn a_copy_when_the_loaded_set_is_full_says_it_is_not_listed() {
        let mut s = with_keys(&["a"]);
        s.keys = LoadedSet::with_cap(1);
        let (s, _) = update(
            s,
            Msg::ScanBatch {
                keys: vec![b"a".to_vec()],
            },
        );
        let (s, _) = settle(s, copy_mutation("a", "z"), MutationOutcome::Done);
        assert_eq!(s.keys.len(), 1);
        assert!(s.notice.as_ref().unwrap().0.contains("not listed"));
    }

    #[test]
    fn a_taken_target_is_an_error_naming_copy_and_marks_nothing() {
        let s = with_keys(&["a"]);
        let (s, _) = settle(
            s,
            copy_mutation("a", "b"),
            MutationOutcome::NotWritten(NotWritten::TargetExists),
        );
        let (text, _) = s.error.as_ref().unwrap();
        assert!(
            text.starts_with("COPY a b:") && text.contains("nothing written"),
            "{text}"
        );
        assert!(!s.keys.is_gone(0));
    }

    #[test]
    fn a_gone_source_marks_the_source_row_gone_and_creates_nothing() {
        let mut s = with_keys(&["a"]);
        s.open = Some(OpenKey::new(
            Some(0),
            "a".into(),
            Value::Str(StringValue::new("v", 1)),
            -1,
            1,
            0,
        ));
        let (s, _) = settle(
            s,
            copy_mutation("a", "b"),
            MutationOutcome::NotWritten(NotWritten::KeyGone),
        );
        assert!(s.keys.is_gone(0));
        assert_eq!(s.keys.len(), 1, "no new key");
        assert!(s.open.as_ref().unwrap().deleted_at_ms.is_some());
        assert!(s.error.as_ref().unwrap().0.contains("COPY a b"));
    }

    #[test]
    fn a_failed_cross_slot_copy_names_dump_restore() {
        let s = cluster(with_keys(&["a"]));
        let (s, _) = update(
            s,
            Msg::MutationSettled {
                mutation: copy_mutation("a", "b"),
                index: Some(0),
                result: Err("boom".into()),
                at_ms: 5_000,
            },
        );
        assert!(s.error.as_ref().unwrap().0.contains("DUMP/RESTORE a b"));
    }
}
