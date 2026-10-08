//! Multi-select and bulk delete (M2 task 13,
//! `docs/plans/m2-task13-bulk-delete.md`): marking, staging the delete,
//! the `prod` typed-count gate, and settling the shell's report.

use super::*;
use crate::mutation::{BulkReport, BulkStop};
use crate::state::scan::thousands;
use crate::state::{BulkDelete, CountGate, Environment};

/// The longest count the typed gate accepts.
const COUNT_DIGITS: usize = 10;

fn notify(state: State, text: impl Into<String>) -> (State, Vec<Command>) {
    (state, vec![Command::Notify { text: text.into() }])
}

/// `Space` with the keys pane focused: mark or unmark the Selected key and
/// move down, so a run of keys is marked by holding the key. O(1) per
/// keystroke (decision 5). A group row cannot be marked: marking a subtree is
/// the high-blast-radius case and is out of scope (decision 1).
pub(super) fn toggle_mark(mut state: State) -> (State, Vec<Command>) {
    if state.row_count() == 0 {
        return (state, Vec::new());
    }
    let Some(index) = state.selected_key() else {
        return notify(state, "a group can't be marked — mark its keys");
    };
    if state.keys.is_gone(index) {
        return notify(state, "gone — nothing to mark");
    }
    state.marks.toggle(index);
    move_selection(state, 1)
}

/// `d` with marks, keys pane focused: stage one `DeleteKeys` over every marked
/// key. Without marks `d` is `delete_selected_key`, untouched.
///
/// Costs O(marked) plus the bitset walk, never a pass over the Loaded set's
/// names. Rows that went gone while marked are dropped: there is nothing left
/// to delete in them, as the single delete already refuses a gone row.
pub(super) fn stage_bulk_delete(mut state: State) -> (State, Vec<Command>) {
    if state.bulk.is_some() {
        return notify(state, "a bulk delete is already running");
    }
    let mut indices = Vec::with_capacity(state.marks.count());
    let mut names = Vec::with_capacity(state.marks.count());
    let mut vanished = Vec::new();
    for i in state.marks.iter() {
        if state.keys.is_gone(i) {
            vanished.push(i);
            continue;
        }
        match state.keys.name(i) {
            Some(name) => {
                indices.push(i as u32);
                names.push(KeyName::from(name));
            }
            None => vanished.push(i),
        }
    }
    for i in vanished {
        state.marks.unmark(i);
    }
    if names.is_empty() {
        return notify(state, "nothing left to delete — the marked keys are gone");
    }
    let gate = if state.connection.environment == Environment::Prod {
        CountGate::Required
    } else {
        CountGate::Open
    };
    state.confirm = Some(PendingMutation::DeleteKeys {
        indices,
        names,
        epoch: state.metadata_epoch,
        gate,
    });
    (state, Vec::new())
}

/// The confirm dialog's keys for a staged bulk delete.
///
/// Elsewhere one `y` confirms. On `prod`, `y` opens the typed gate and the
/// count must be typed exactly and submitted with `Enter` (a count such as
/// `50` is a prefix of `500`, so matching alone must not confirm). `Esc`
/// dismisses at any stage and every other key is ignored, so a stray key can
/// never throw the staging away (ADR-0014). Read-only Mode refuses at the
/// first `y`, after the dialog has shown the real command (DESIGN §6.5).
pub(super) fn bulk_confirm_key(
    mut state: State,
    pending: PendingMutation,
    key: KeyPress,
) -> (State, Vec<Command>) {
    let PendingMutation::DeleteKeys {
        indices,
        names,
        epoch,
        mut gate,
    } = pending
    else {
        unreachable!("bulk_confirm_key is only called for DeleteKeys");
    };
    let plain = !key.ctrl && !key.alt;
    let mut run = false;
    match (&mut gate, key.code) {
        (_, KeyCode::Esc) => {
            clear_editing(&mut state);
            return (state, Vec::new());
        }
        (CountGate::Open, KeyCode::Char('y')) if plain => run = true,
        (CountGate::Required, KeyCode::Char('y')) if plain => {
            if refuse_if_read_only(&mut state) {
                return notify_read_only(state);
            }
            gate = CountGate::Typing {
                text: String::new(),
                wrong: false,
            };
        }
        (CountGate::Typing { text, wrong }, KeyCode::Char(c)) if plain && c.is_ascii_digit() => {
            if text.len() < COUNT_DIGITS {
                text.push(c);
            }
            *wrong = false;
        }
        (CountGate::Typing { text, wrong }, KeyCode::Backspace) => {
            text.pop();
            *wrong = false;
        }
        (CountGate::Typing { text, wrong }, KeyCode::Enter) => {
            if text.parse::<usize>().ok() == Some(names.len()) {
                run = true;
            } else {
                *wrong = true;
            }
        }
        _ => {}
    }
    if run {
        if refuse_if_read_only(&mut state) {
            return notify_read_only(state);
        }
        state.bulk = Some(BulkDelete {
            total: names.len(),
            done: 0,
            cancelling: false,
            epoch,
            indices: indices.clone(),
        });
        let command = PendingMutation::DeleteKeys {
            indices,
            names,
            epoch,
            gate: CountGate::Open,
        }
        .into_command();
        return (state, vec![command]);
    }
    state.confirm = Some(PendingMutation::DeleteKeys {
        indices,
        names,
        epoch,
        gate,
    });
    (state, Vec::new())
}

fn refuse_if_read_only(state: &mut State) -> bool {
    if state.read_only.is_some() {
        clear_editing(state);
        true
    } else {
        false
    }
}

fn notify_read_only(state: State) -> (State, Vec<Command>) {
    let reason = state.read_only.map(|r| r.label()).unwrap_or_default();
    notify(state, format!("read-only ({reason}): not executed"))
}

/// `Msg::BulkDeleteProgress`.
pub(super) fn bulk_progress(mut state: State, done: usize, total: usize) -> (State, Vec<Command>) {
    if let Some(bulk) = &mut state.bulk
        && bulk.total == total
    {
        bulk.done = done.min(total);
    }
    (state, Vec::new())
}

/// `Esc` while a bulk delete runs: ask the shell to stop after the batch in
/// flight. A second `Esc` has nothing more to say.
pub(super) fn cancel_bulk(mut state: State) -> (State, Vec<Command>) {
    match state.bulk.as_mut() {
        Some(bulk) if !bulk.cancelling => {
            bulk.cancelling = true;
            (state, vec![Command::CancelBulkDelete])
        }
        _ => (state, Vec::new()),
    }
}

/// A key the server no longer has: its row is badged and, if it is the Open
/// key, the Viewer tombstones it with its last value kept (ADR-0006). Shared by
/// the single delete and the bulk one, so both end the same way.
pub(super) fn mark_deleted(state: &mut State, index: Option<usize>, name: &KeyName, at_ms: u64) {
    if let Some(index) = index
        && state.keys.name(index) == Some(name.as_bytes())
    {
        state.keys.set_gone(index);
    }
    if state.open.as_ref().is_some_and(|o| o.name == *name) {
        if let Some(open) = &mut state.open {
            open.deleted_at_ms = Some(at_ms);
            open.pending = None;
        }
        staged_edit_found_key_gone(state, name, at_ms);
    }
}

/// `MutationSettled` for a `DeleteKeys`: badge what was deleted, unmark what
/// was processed, and say how it ended (R7.4).
pub(super) fn bulk_settled(
    mut state: State,
    keys: &[KeyName],
    report: BulkReport,
    at_ms: u64,
) -> (State, Vec<Command>) {
    let bulk = state.bulk.take();
    let current = bulk
        .as_ref()
        .filter(|b| b.epoch == state.metadata_epoch && b.indices.len() == keys.len());
    let positions = (0..report.processed.min(keys.len()))
        .chain(report.extra_ok.iter().map(|p| *p as usize))
        .filter(|p| *p < keys.len())
        .collect::<Vec<_>>();
    for pos in positions {
        let index = current.map(|b| b.indices[pos] as usize);
        mark_deleted(&mut state, index, &keys[pos], at_ms);
        if let Some(index) = index {
            state.marks.unmark(index);
        }
    }
    let counts = format!(
        "{} of {} keys",
        thousands(report.deleted as u64),
        thousands(report.total as u64)
    );
    let gone = if report.already_gone > 0 {
        format!(" ({} already gone)", thousands(report.already_gone as u64))
    } else {
        String::new()
    };
    match report.stopped {
        BulkStop::Completed => {
            state.notice = Some((format!("deleted {counts}{gone}"), at_ms));
            (state, Vec::new())
        }
        BulkStop::Cancelled => {
            state.notice = Some((
                format!("cancelled: deleted {counts}{gone} · the rest stay marked"),
                at_ms,
            ));
            (state, Vec::new())
        }
        BulkStop::Failed { command, detail } => {
            let detail = format!(
                "{detail} — stopped after {} of {} keys ({} deleted{})",
                thousands(report.processed as u64),
                thousands(report.total as u64),
                thousands(report.deleted as u64),
                if report.already_gone > 0 {
                    format!(", {} already gone", thousands(report.already_gone as u64))
                } else {
                    String::new()
                }
            );
            failed(state, command, detail, at_ms)
        }
    }
}
