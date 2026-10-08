//! Multi-select and bulk delete through `update` (M2 task 13,
//! `docs/plans/m2-task13-bulk-delete.md`).

use super::*;
use crate::msg::KeyCode;
use crate::mutation::{BulkReport, BulkStop};
use crate::state::{CountGate, Environment, ReadOnlyReason};

const SLICE: usize = 8;

fn names(n: usize) -> Vec<Vec<u8>> {
    (0..n).map(|i| format!("k{i:04}").into_bytes()).collect()
}

/// A scanned, complete, built flat list of `n` keys (`k0000`, `k0001`, …).
fn loaded(n: usize, env: Environment) -> State {
    let mut state = State {
        cols: 130,
        rows: 30,
        rebuild_slice: Some(SLICE),
        ..State::default()
    };
    state.connection.environment = env;
    let (state, _) = update(
        state,
        Msg::ScanStarted {
            estimated_total: n as u64,
        },
    );
    let (state, _) = update(state, Msg::ScanBatch { keys: names(n) });
    let (state, _) = update(state, Msg::ScanComplete);
    settle(state)
}

fn settle(mut state: State) -> State {
    for _ in 0..100_000 {
        if !state.rebuild_running() {
            return state;
        }
        (state, _) = update(state, Msg::RebuildStep);
    }
    panic!("job never finished");
}

fn press(state: State, c: char) -> (State, Vec<Command>) {
    update(state, Msg::Key(KeyPress::plain(KeyCode::Char(c))))
}

fn code(state: State, code: KeyCode) -> (State, Vec<Command>) {
    update(state, Msg::Key(KeyPress::plain(code)))
}

/// Mark rows `rows` (display rows of a flat list) with `Space`.
fn mark_rows(mut state: State, rows: &[usize]) -> State {
    for &r in rows {
        state.view.selected = r;
        (state, _) = press(state, ' ');
    }
    state
}

fn marked(state: &State) -> Vec<usize> {
    state.marks.iter().collect()
}

fn notice_text(commands: &[Command]) -> Option<&str> {
    commands.iter().find_map(|c| match c {
        Command::Notify { text } => Some(text.as_str()),
        _ => None,
    })
}

fn staged(state: &State) -> (&[u32], &[crate::key::KeyName], &CountGate) {
    match &state.confirm {
        Some(PendingMutation::DeleteKeys {
            indices,
            names,
            gate,
            ..
        }) => (indices, names, gate),
        other => panic!("expected a staged bulk delete, got {other:?}"),
    }
}

fn executed(commands: &[Command]) -> Option<&Mutation> {
    commands.iter().find_map(|c| match c {
        Command::Execute { mutation, .. } => Some(mutation),
        _ => None,
    })
}

// ── marking ─────────────────────────────────────────────────────────────────

#[test]
fn space_toggles_a_mark_and_moves_down() {
    let state = loaded(20, Environment::Local);
    let (state, _) = press(state, ' ');
    assert_eq!(marked(&state), vec![0]);
    assert_eq!(state.view.selected, 1);
    let (state, _) = code(state, KeyCode::Up);
    let (state, _) = press(state, ' ');
    assert!(state.marks.is_empty(), "the second Space unmarks");
    assert_eq!(state.view.selected, 1);
}

#[test]
fn space_on_the_last_row_marks_it_and_stays() {
    let mut state = loaded(3, Environment::Local);
    state.view.selected = 2;
    let (state, _) = press(state, ' ');
    assert_eq!(marked(&state), vec![2]);
    assert_eq!(state.view.selected, 2);
}

#[test]
fn space_on_a_group_row_says_so_and_marks_nothing() {
    let mut state = State {
        cols: 130,
        rows: 30,
        tree_mode: true,
        ..State::default()
    };
    (state, _) = update(state, Msg::ScanStarted { estimated_total: 2 });
    (state, _) = update(
        state,
        Msg::ScanBatch {
            keys: vec![b"a:1".to_vec(), b"a:2".to_vec()],
        },
    );
    state = settle(state);
    assert!(state.selected_key().is_none(), "row 0 is the group");
    let (state, commands) = press(state, ' ');
    assert!(state.marks.is_empty());
    assert!(notice_text(&commands).unwrap().contains("group"));
}

#[test]
fn space_on_a_gone_row_is_refused() {
    let mut state = loaded(3, Environment::Local);
    state.keys.set_gone(0);
    let (state, commands) = press(state, ' ');
    assert!(state.marks.is_empty());
    assert!(notice_text(&commands).unwrap().contains("gone"));
}

#[test]
fn space_does_nothing_in_the_value_pane() {
    let mut state = loaded(3, Environment::Local);
    state.focus = Pane::Value;
    let (state, _) = press(state, ' ');
    assert!(state.marks.is_empty());
}

#[test]
fn space_on_an_empty_list_does_nothing() {
    let (state, commands) = press(State::default(), ' ');
    assert!(state.marks.is_empty() && commands.is_empty());
}

// ── marks survive the things that reshape the view ──────────────────────────

#[test]
fn marks_survive_filter_sort_and_tree_toggle() {
    let state = loaded(40, Environment::Local);
    let state = mark_rows(state, &[3, 7, 11]);
    let want = marked(&state);
    assert_eq!(want.len(), 3);

    // Filter to a handful of rows, then clear it.
    let (state, _) = press(state, '/');
    let mut state = state;
    for c in "k003".chars() {
        (state, _) = press(state, c);
    }
    (state, _) = code(state, KeyCode::Enter);
    let state = settle(state);
    assert_eq!(marked(&state), want, "filtering does not renumber");
    let (state, _) = press(state, '/');
    let mut state = state;
    for _ in 0..4 {
        (state, _) = code(state, KeyCode::Backspace);
    }
    (state, _) = code(state, KeyCode::Enter);
    let state = settle(state);

    let (state, _) = press(state, 's');
    let state = settle(state);
    assert_eq!(marked(&state), want, "sorting does not renumber");
    let (state, _) = press(state, 't');
    let state = settle(state);
    assert_eq!(marked(&state), want, "the tree toggle does not renumber");
    let (state, _) = press(state, 't');
    let state = settle(state);
    assert_eq!(marked(&state), want);
}

#[test]
fn marks_survive_a_sliced_rebuild_job_and_a_row_marked_during_one_counts() {
    let state = loaded(100, Environment::Local);
    let state = mark_rows(state, &[2, 50]);
    // A sort change over 100 keys with a slice of 8 is a job.
    let (state, _) = press(state, 's');
    assert!(state.rebuild_running(), "the sort is a sliced job");
    assert_eq!(marked(&state).len(), 2, "marks are intact mid-job");
    // Marking while the old list is still shown works and is keyed by index.
    let mut state = state;
    state.view.selected = 5;
    let index = state.selected_key().unwrap();
    let (state, _) = press(state, ' ');
    assert!(state.marks.is_marked(index));
    let state = settle(state);
    assert!(!state.rebuild_running());
    assert_eq!(state.marks.count(), 3, "the swap leaves every mark alone");
}

#[test]
fn a_rescan_clears_the_marks_where_the_epoch_bumps() {
    let state = mark_rows(loaded(10, Environment::Local), &[1, 2]);
    let epoch = state.metadata_epoch;
    let (state, _) = update(
        state,
        Msg::ScanStarted {
            estimated_total: 10,
        },
    );
    assert_ne!(state.metadata_epoch, epoch);
    assert!(state.marks.is_empty());
}

// ── Esc ─────────────────────────────────────────────────────────────────────

#[test]
fn the_first_esc_clears_marks_and_a_scan_is_cancelled_only_by_the_next() {
    let mut state = mark_rows(loaded(10, Environment::Local), &[1, 2]);
    state.scan = crate::state::ScanState::Running {
        scanned: 5,
        estimated_total: 10,
    };
    let (state, commands) = code(state, KeyCode::Esc);
    assert!(state.marks.is_empty());
    assert!(commands.is_empty(), "the scan is left running");
    let (_, commands) = code(state, KeyCode::Esc);
    assert!(commands.iter().any(|c| matches!(c, Command::CancelScan)));
}

#[test]
fn esc_cancels_a_running_bulk_delete_before_it_clears_marks() {
    let state = mark_rows(loaded(10, Environment::Local), &[1, 2]);
    let (state, _) = press(state, 'd');
    let (state, _) = press(state, 'y');
    assert!(state.bulk.is_some());
    let (state, commands) = code(state, KeyCode::Esc);
    assert!(
        commands
            .iter()
            .any(|c| matches!(c, Command::CancelBulkDelete))
    );
    assert!(state.bulk.as_ref().unwrap().cancelling);
    assert_eq!(state.marks.count(), 2, "marks are left for the report");
    let (_, commands) = code(state, KeyCode::Esc);
    assert!(commands.is_empty(), "a second Esc asks for nothing more");
}

// ── staging ─────────────────────────────────────────────────────────────────

#[test]
fn d_without_marks_is_the_single_key_delete_unchanged() {
    let state = loaded(5, Environment::Local);
    let (state, commands) = press(state, 'd');
    assert!(commands.is_empty());
    assert!(matches!(
        state.confirm,
        Some(PendingMutation::DeleteKey { index: 0, .. })
    ));
}

#[test]
fn d_with_marks_stages_one_bulk_delete_in_ascending_order() {
    let state = mark_rows(loaded(30, Environment::Local), &[9, 2, 20]);
    let (state, commands) = press(state, 'd');
    assert!(commands.is_empty(), "nothing runs before it is confirmed");
    let (indices, names, gate) = staged(&state);
    assert_eq!(indices, &[2, 9, 20]);
    assert_eq!(
        names.iter().map(|n| n.to_string()).collect::<Vec<_>>(),
        vec!["k0002", "k0009", "k0020"]
    );
    assert_eq!(gate, &CountGate::Open);
    let pending = state.confirm.as_ref().unwrap();
    assert_eq!(pending.command_text(), "DEL × 3 keys");
    assert!(pending.guard_text().unwrap().contains("one DEL per key"));
}

#[test]
fn the_preview_lists_the_first_names_and_how_many_more() {
    let rows: Vec<usize> = (0..12).collect();
    let state = mark_rows(loaded(30, Environment::Local), &rows);
    let (state, _) = press(state, 'd');
    let text = crate::render::to_text(&crate::render::frame(
        &state,
        &crate::theme::Theme::new(crate::theme::ColorDepth::Monochrome),
        &crate::clock::FixedClock(0),
        ratatui::layout::Rect::new(0, 0, 100, 30),
    ));
    assert!(text.contains("DEL × 12 keys"), "{text}");
    let listed = text.lines().filter(|l| l.contains("│   k00")).count();
    assert_eq!(listed, 8, "the first eight names only:\n{text}");
    assert!(text.contains("… and 4 more"), "{text}");
}

#[test]
fn rows_that_went_gone_while_marked_are_dropped_at_staging() {
    let mut state = mark_rows(loaded(10, Environment::Local), &[1, 2, 3]);
    state.keys.set_gone(2);
    let (state, _) = press(state, 'd');
    let (indices, _, _) = staged(&state);
    assert_eq!(indices, &[1, 3]);
    assert_eq!(state.marks.count(), 2, "the gone mark is released");
}

#[test]
fn all_marked_rows_gone_stages_nothing() {
    let mut state = mark_rows(loaded(10, Environment::Local), &[1]);
    state.keys.set_gone(1);
    let (state, commands) = press(state, 'd');
    assert!(state.confirm.is_none());
    assert!(notice_text(&commands).unwrap().contains("gone"));
    assert!(state.marks.is_empty());
}

#[test]
fn d_with_marks_in_the_value_pane_is_not_a_bulk_delete() {
    let mut state = mark_rows(loaded(10, Environment::Local), &[1]);
    state.focus = Pane::Value;
    let (state, _) = press(state, 'd');
    assert!(!matches!(
        state.confirm,
        Some(PendingMutation::DeleteKeys { .. })
    ));
}

#[test]
fn d_while_a_bulk_delete_runs_says_so() {
    let state = mark_rows(loaded(10, Environment::Local), &[1, 2]);
    let (state, _) = press(state, 'd');
    let (state, _) = press(state, 'y');
    let (state, commands) = press(state, 'd');
    assert!(state.confirm.is_none());
    assert!(notice_text(&commands).unwrap().contains("already running"));
}

// ── confirming ──────────────────────────────────────────────────────────────

#[test]
fn off_prod_one_y_executes() {
    for env in [Environment::Local, Environment::Staging] {
        let state = mark_rows(loaded(10, env), &[1, 2]);
        let (state, _) = press(state, 'd');
        let (state, commands) = press(state, 'y');
        match executed(&commands) {
            Some(Mutation::DeleteKeys { keys }) => assert_eq!(keys.len(), 2),
            other => panic!("expected DeleteKeys, got {other:?}"),
        }
        assert!(state.confirm.is_none());
        let bulk = state.bulk.unwrap();
        assert_eq!((bulk.total, bulk.done), (2, 0));
        assert_eq!(bulk.indices, vec![1, 2]);
    }
}

#[test]
fn off_prod_other_keys_are_ignored_and_esc_dismisses() {
    let state = mark_rows(loaded(10, Environment::Local), &[1, 2]);
    let (state, _) = press(state, 'd');
    let (state, commands) = press(state, 'x');
    assert!(commands.is_empty() && state.confirm.is_some());
    let (state, commands) = code(state, KeyCode::Esc);
    assert!(commands.is_empty() && state.confirm.is_none());
    assert_eq!(state.marks.count(), 2, "dismissing keeps the marks");
}

fn prod_dialog(n: usize) -> State {
    let rows: Vec<usize> = (0..n).collect();
    let mut state = mark_rows(loaded(n.max(10), Environment::Prod), &rows);
    state.read_only = None; // lifted, as a reader would to write on prod
    let (state, _) = press(state, 'd');
    state
}

fn type_digits(mut state: State, digits: &str) -> State {
    for c in digits.chars() {
        (state, _) = press(state, c);
    }
    state
}

#[test]
fn on_prod_y_opens_the_typed_gate_and_runs_nothing() {
    let state = prod_dialog(12);
    assert_eq!(staged(&state).2, &CountGate::Required);
    let (state, commands) = press(state, 'y');
    assert!(commands.is_empty());
    assert!(
        matches!(staged(&state).2, CountGate::Typing { text, wrong: false } if text.is_empty())
    );
}

#[test]
fn on_prod_digits_are_typed_and_nothing_else_is() {
    let state = prod_dialog(12);
    let (state, _) = press(state, 'y');
    let state = type_digits(state, "1x2 y");
    match staged(&state).2 {
        CountGate::Typing { text, .. } => assert_eq!(text, "12"),
        other => panic!("{other:?}"),
    }
    let (state, _) = code(state, KeyCode::Backspace);
    match staged(&state).2 {
        CountGate::Typing { text, .. } => assert_eq!(text, "1"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn on_prod_a_wrong_count_keeps_the_dialog_open() {
    let state = prod_dialog(12);
    let (state, _) = press(state, 'y');
    let state = type_digits(state, "11");
    let (state, commands) = code(state, KeyCode::Enter);
    assert!(commands.is_empty() && state.bulk.is_none());
    assert!(matches!(
        staged(&state).2,
        CountGate::Typing { wrong: true, .. }
    ));
    // Empty is wrong too, and an edit clears the complaint.
    let (state, _) = code(state, KeyCode::Backspace);
    assert!(matches!(
        staged(&state).2,
        CountGate::Typing { wrong: false, .. }
    ));
    let (state, _) = code(state, KeyCode::Backspace);
    let (state, commands) = code(state, KeyCode::Enter);
    assert!(commands.is_empty());
    assert!(matches!(
        staged(&state).2,
        CountGate::Typing { wrong: true, .. }
    ));
}

#[test]
fn on_prod_a_prefix_of_the_count_does_not_confirm_by_itself() {
    // 12 marked: typing "1" matches nothing, and typing the exact "12" does
    // not run until Enter; the gate never auto-submits.
    let state = prod_dialog(12);
    let (state, _) = press(state, 'y');
    let (state, commands) = press(state, '1');
    assert!(commands.is_empty());
    let (state, commands) = press(state, '2');
    assert!(commands.is_empty() && state.bulk.is_none());
}

#[test]
fn on_prod_only_the_exact_count_then_enter_executes() {
    let state = prod_dialog(12);
    let (state, _) = press(state, 'y');
    let state = type_digits(state, "12");
    let (state, commands) = code(state, KeyCode::Enter);
    assert!(matches!(
        executed(&commands),
        Some(Mutation::DeleteKeys { keys }) if keys.len() == 12
    ));
    assert_eq!(state.bulk.unwrap().total, 12);
}

#[test]
fn on_prod_y_while_typing_does_not_confirm_and_esc_dismisses() {
    let state = prod_dialog(12);
    let (state, _) = press(state, 'y');
    let (state, commands) = press(state, 'y');
    assert!(commands.is_empty() && state.confirm.is_some());
    let (state, commands) = code(state, KeyCode::Esc);
    assert!(commands.is_empty() && state.confirm.is_none() && state.bulk.is_none());
    assert_eq!(state.marks.count(), 12);
}

#[test]
fn read_only_refuses_at_confirm_after_the_dialog_is_composed() {
    // Off prod: the first y refuses.
    let mut state = mark_rows(loaded(10, Environment::Staging), &[1, 2]);
    state.read_only = Some(ReadOnlyReason::User);
    let (state, commands) = press(state, 'd');
    assert!(state.confirm.is_some(), "the dialog is shown first");
    assert!(commands.is_empty());
    let (state, commands) = press(state, 'y');
    assert!(executed(&commands).is_none() && state.bulk.is_none());
    assert!(notice_text(&commands).unwrap().contains("read-only (user)"));
    assert!(state.confirm.is_none());

    // On prod with the default Environment guard: refused at the first y, so
    // the reader is never asked to type a count for something that cannot run.
    let mut state = mark_rows(loaded(10, Environment::Prod), &[1, 2]);
    state.read_only = Some(ReadOnlyReason::Environment);
    let (state, _) = press(state, 'd');
    let (state, commands) = press(state, 'y');
    assert!(executed(&commands).is_none() && state.confirm.is_none());
    assert!(
        notice_text(&commands)
            .unwrap()
            .contains("read-only (environment)")
    );
}

// ── settling ────────────────────────────────────────────────────────────────

fn settle_bulk(
    state: State,
    keys: Vec<crate::key::KeyName>,
    report: BulkReport,
) -> (State, Vec<Command>) {
    update(
        state,
        Msg::MutationSettled {
            mutation: Mutation::DeleteKeys { keys },
            index: None,
            result: Ok(MutationOutcome::BulkDeleted(report)),
            at_ms: 5_000,
        },
    )
}

fn running(n: usize, rows: &[usize]) -> (State, Vec<crate::key::KeyName>) {
    let state = mark_rows(loaded(n, Environment::Local), rows);
    let (state, _) = press(state, 'd');
    let keys = staged(&state).1.to_vec();
    let (state, _) = press(state, 'y');
    (state, keys)
}

fn report(total: usize, processed: usize, deleted: usize, stopped: BulkStop) -> BulkReport {
    BulkReport {
        total,
        processed,
        deleted,
        already_gone: processed - deleted,
        extra_ok: Vec::new(),
        stopped,
    }
}

#[test]
fn progress_is_recorded() {
    let (state, _) = running(10, &[1, 2, 3]);
    let (state, _) = update(state, Msg::BulkDeleteProgress { done: 2, total: 3 });
    assert_eq!(state.bulk.unwrap().done, 2);
}

#[test]
fn a_completed_delete_badges_every_row_clears_the_marks_and_counts() {
    let (state, keys) = running(10, &[1, 2, 3]);
    let (state, _) = settle_bulk(state, keys, report(3, 3, 3, BulkStop::Completed));
    for i in [1, 2, 3] {
        assert!(state.keys.is_gone(i));
    }
    assert!(!state.keys.is_gone(0) && !state.keys.is_gone(4));
    assert!(state.marks.is_empty() && state.bulk.is_none());
    assert_eq!(state.notice.as_ref().unwrap().0, "deleted 3 of 3 keys");
}

#[test]
fn keys_already_gone_are_counted_as_such() {
    let (state, keys) = running(10, &[1, 2, 3]);
    let (state, _) = settle_bulk(state, keys, report(3, 3, 1, BulkStop::Completed));
    assert_eq!(
        state.notice.as_ref().unwrap().0,
        "deleted 1 of 3 keys (2 already gone)"
    );
    assert!(state.keys.is_gone(2), "an already-gone key is badged too");
}

#[test]
fn a_cancelled_delete_badges_the_processed_prefix_and_keeps_the_rest_marked() {
    let (state, keys) = running(10, &[1, 2, 3, 4]);
    let (state, _) = code(state, KeyCode::Esc);
    let (state, _) = settle_bulk(state, keys, report(4, 2, 2, BulkStop::Cancelled));
    assert!(state.keys.is_gone(1) && state.keys.is_gone(2));
    assert!(!state.keys.is_gone(3) && !state.keys.is_gone(4));
    assert_eq!(marked(&state), vec![3, 4]);
    let notice = &state.notice.as_ref().unwrap().0;
    assert!(
        notice.starts_with("cancelled: deleted 2 of 4 keys"),
        "{notice}"
    );
    assert!(state.bulk.is_none());
}

#[test]
fn a_failed_delete_names_the_command_and_how_far_it_got() {
    let (state, keys) = running(10, &[1, 2, 3, 4, 5]);
    let mut r = report(
        5,
        2,
        2,
        BulkStop::Failed {
            command: "DEL k0003".into(),
            detail: "READONLY You can't write against a read only replica.".into(),
        },
    );
    // Position 4 succeeded in the failing batch, after the first error at 2.
    r.extra_ok = vec![4];
    r.deleted += 1;
    let (state, _) = settle_bulk(state, keys, r);
    let error = &state.error.as_ref().unwrap().0;
    assert!(error.starts_with("DEL k0003: READONLY"), "{error}");
    assert!(
        error.contains("stopped after 2 of 5 keys (3 deleted)"),
        "{error}"
    );
    assert!(state.keys.is_gone(1) && state.keys.is_gone(2));
    assert!(!state.keys.is_gone(3), "the failed key stays live");
    assert!(state.keys.is_gone(5), "a later success is still badged");
    assert_eq!(marked(&state), vec![3, 4]);
}

#[test]
fn a_rescan_while_deleting_leaves_the_new_numbering_alone() {
    let (state, keys) = running(10, &[1, 2]);
    let (state, _) = update(
        state,
        Msg::ScanStarted {
            estimated_total: 10,
        },
    );
    let (state, _) = update(state, Msg::ScanBatch { keys: names(10) });
    let (state, _) = settle_bulk(state, keys, report(2, 2, 2, BulkStop::Completed));
    assert!(
        !state.keys.is_gone(1) && !state.keys.is_gone(2),
        "indices from the old numbering are not used"
    );
    assert!(state.bulk.is_none());
}

#[test]
fn a_bulk_delete_of_the_open_key_tombstones_it_like_a_single_delete() {
    let (mut state, keys) = running(10, &[1, 2]);
    state.open = Some(crate::state::OpenKey::new(
        Some(2),
        keys[1].clone(),
        crate::state::Value::Str(crate::state::value::StringValue::new("v", 40)),
        -1,
        1,
        10,
    ));
    let (state, _) = settle_bulk(state, keys, report(2, 2, 2, BulkStop::Completed));
    assert_eq!(state.open.as_ref().unwrap().deleted_at_ms, Some(5_000));
}

#[test]
fn a_failure_before_any_batch_settles_as_an_ordinary_error() {
    let (state, keys) = running(10, &[1, 2]);
    let (state, _) = update(
        state,
        Msg::MutationSettled {
            mutation: Mutation::DeleteKeys { keys },
            index: None,
            result: Err("connection reset".into()),
            at_ms: 9,
        },
    );
    assert_eq!(
        state.error.as_ref().unwrap().0,
        "DEL (2 keys): connection reset"
    );
    assert!(!state.keys.is_gone(1));
    assert!(state.bulk.is_none(), "the in-flight record ends with it");
}
