//! The rebuild job's lifecycle through `update` (M6 task 3,
//! `docs/plans/m6-rebuild-job.md` decision 5): which triggers start, replace,
//! coalesce into or abort a running job, and that every path ends where the
//! synchronous rebuild does. The engine's own equivalence is in
//! `state::rebuild`.

use super::*;
use crate::msg::KeyCode;
use crate::state::SortBy;
use crate::state::rebuild::tests::{Rng, assert_same, pick, random_name, random_state};

const SLICE: usize = 8;

fn key_names(n: usize, seed: u64) -> Vec<Vec<u8>> {
    let mut rng = Rng(seed);
    (0..n)
        .map(|i| format!("{}:{i}", random_name(&mut rng)).into_bytes())
        .collect()
}

/// A scanned, complete, built list of `n` keys with a small slice, so a rebuild
/// is a job. Flat unless `tree`.
fn loaded(n: usize, tree: bool) -> State {
    let state = State {
        cols: 130,
        rows: 30,
        tree_mode: tree,
        rebuild_slice: Some(SLICE),
        ..State::default()
    };
    let (state, _) = update(
        state,
        Msg::ScanStarted {
            estimated_total: n as u64,
        },
    );
    let (state, _) = update(
        state,
        Msg::ScanBatch {
            keys: key_names(n, 5),
        },
    );
    let (state, _) = update(state, Msg::ScanComplete);
    settle(state)
}

/// Run any job to its swap through `Msg::RebuildStep`, as the shell would.
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

fn shown(state: &State) -> Vec<Option<usize>> {
    (0..state.row_count()).map(|r| state.key_at(r)).collect()
}

/// `state` with the same operations done synchronously: no job can start.
fn sync_twin(state: &State) -> State {
    State {
        rebuild_slice: Some(usize::MAX),
        job: None,
        ..state.clone()
    }
}

#[test]
fn a_large_sort_change_starts_a_job_and_the_old_list_stays_usable() {
    let state = loaded(100, false);
    let before = shown(&state);
    let (state, commands) = press(state, 's');
    assert!(state.rebuild_running());
    assert!(commands.contains(&Command::ContinueRebuild));
    assert_eq!(shown(&state), before, "the old list is what is on screen");
    // Navigation works on it, and survives to the swap.
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Down)));
    assert_eq!(state.view.selected, 1);
    let want = {
        let mut twin = sync_twin(&state);
        twin.rebuild_list();
        twin
    };
    let state = settle(state);
    assert_eq!(state.list, want.list);
    assert_eq!(
        state.key_at(state.view.selected),
        want.key_at(want.view.selected)
    );
}

#[test]
fn a_small_keyspace_never_starts_a_job() {
    let state = State {
        cols: 130,
        rows: 30,
        ..State::default()
    };
    let (state, _) = update(state, Msg::ScanStarted { estimated_total: 5 });
    let (state, _) = update(
        state,
        Msg::ScanBatch {
            keys: key_names(5, 1),
        },
    );
    let (state, commands) = press(state, 's');
    assert!(!state.rebuild_running());
    assert!(!commands.contains(&Command::ContinueRebuild));
    assert_eq!(state.list.sort, SortBy::Name);
}

#[test]
fn continue_is_asked_for_exactly_while_a_job_runs() {
    let (state, commands) = press(loaded(100, false), 's');
    assert!(commands.contains(&Command::ContinueRebuild));
    // A message that has nothing to do with the job still asks.
    let (state, commands) = update(state, Msg::DashboardPollTick);
    assert!(commands.contains(&Command::ContinueRebuild));
    let state = settle(state);
    let (_, commands) = update(state, Msg::DashboardPollTick);
    assert!(!commands.contains(&Command::ContinueRebuild));
}

#[test]
fn a_step_with_no_job_is_ignored() {
    let state = loaded(100, false);
    let (after, commands) = update(state.clone(), Msg::RebuildStep);
    assert_eq!(after, state);
    assert!(commands.is_empty());
}

#[test]
fn a_user_trigger_replaces_a_running_job() {
    let (mut state, _) = press(loaded(100, false), 's');
    for _ in 0..3 {
        (state, _) = update(state, Msg::RebuildStep);
    }
    assert!(state.rebuild_running());
    let (state, _) = press(state, 's');
    let job = state.job.as_ref().expect("a fresh job");
    assert_eq!(state.rebuild_progress(), Some(0), "it started over");
    assert_eq!(job.covered(), 100);
    assert_eq!(
        state.list.sort,
        SortBy::Ttl,
        "the second press is the target"
    );
    let mut want = sync_twin(&state);
    want.rebuild_list();
    let state = settle(state);
    assert_eq!(state.list, want.list);
}

#[test]
fn a_scan_page_marks_the_job_dirty_and_never_replaces_it() {
    let (mut state, _) = press(loaded(100, false), 's');
    (state, _) = update(state, Msg::RebuildStep);
    (state, _) = update(state, Msg::RebuildStep);
    let rows = shown(&state);
    let page = key_names(100, 77)
        .into_iter()
        .map(|mut k| {
            k.extend_from_slice(b"+late");
            k
        })
        .collect::<Vec<_>>();
    let mut twin = sync_twin(&state);
    let (mut state, _) = update(state, Msg::ScanBatch { keys: page.clone() });
    assert!(state.rebuild_running(), "a page never replaces a job");
    assert!(state.job.as_ref().unwrap().is_dirty());
    assert_eq!(shown(&state), rows, "the shown list is untouched by a page");
    assert_eq!(state.keys.len(), 200);
    state = settle(state);
    // 100% growth: the follow-up job ran and the list is level.
    assert_eq!(state.list.covered_len(), 200);
    (twin, _) = update(twin, Msg::ScanBatch { keys: page });
    twin.rebuild_list();
    assert_eq!(state.list, twin.list);
}

#[test]
fn narrowing_cancels_a_running_job() {
    let mut state = loaded(100, false);
    state.rebuild_list_async();
    assert!(state.rebuild_running());
    state.filtering = true;
    let (state, _) = press(state, 'a');
    assert!(!state.rebuild_running(), "the narrowed list is current");
    assert!(state.list.is_current_for(state.keys.len()));
}

#[test]
fn a_rescan_aborts_the_job_and_nothing_is_swapped() {
    let (mut state, _) = press(loaded(100, false), 's');
    (state, _) = update(state, Msg::RebuildStep);
    let (state, _) = update(state, Msg::ScanStarted { estimated_total: 0 });
    assert!(!state.rebuild_running());
    assert!(state.keys.is_empty());
    assert_eq!(state.row_count(), 0);
    let (_, commands) = update(state, Msg::RebuildStep);
    assert!(commands.is_empty(), "a late step is ignored");
}

#[test]
fn the_cap_restarts_the_job_over_every_key() {
    let mut state = loaded(100, false);
    state.keys = {
        let mut k = crate::state::LoadedSet::with_cap(130);
        for key in key_names(100, 5) {
            k.push(&key);
        }
        k
    };
    state.rebuild_list();
    let (state, _) = press(state, 's');
    assert!(state.rebuild_running());
    let (state, commands) = update(
        state,
        Msg::ScanBatch {
            keys: key_names(100, 9),
        },
    );
    assert!(commands.contains(&Command::CancelScan));
    assert_eq!(state.keys.len(), 130);
    let job = state
        .job
        .as_ref()
        .expect("restarted, not left to finish short");
    assert_eq!(job.covered(), 130);
    assert!(!job.is_dirty());
    let state = settle(state);
    assert_eq!(state.list.covered_len(), 130);
}

#[test]
fn scan_end_replaces_a_running_job_so_the_list_is_whole() {
    let (mut state, _) = press(loaded(100, false), 's');
    (state, _) = update(state, Msg::RebuildStep);
    (state, _) = update(
        state,
        Msg::ScanBatch {
            keys: key_names(10, 3),
        },
    );
    let (state, _) = update(state, Msg::ScanComplete);
    assert_eq!(state.job.as_ref().map(|j| j.covered()), Some(110));
    assert!(!state.job.as_ref().unwrap().is_dirty());
    let state = settle(state);
    assert_eq!(state.list.covered_len(), 110);
}

#[test]
fn a_collapse_during_a_job_replaces_it_and_a_second_press_is_not_an_expand() {
    let mut state = loaded(100, true);
    state.view.selected = 0;
    assert!(matches!(
        state.tree.row(0),
        Some(crate::state::tree::Row::Group { expanded: true, .. })
    ));
    let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Left)));
    assert_eq!(
        state.job.as_ref().map(|j| j.kind()),
        Some(crate::state::JobKind::FoldOnly)
    );
    let before = state.tree.len();
    let (mut state, _) = update(state, Msg::RebuildStep);
    // The rows still show the group open; a second Left must not re-expand it.
    let prefix = super::keys::group_prefix_at(&state, 0).unwrap();
    (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Left)));
    assert!(state.tree.is_collapsed(&prefix), "still collapsed");
    assert_eq!(state.tree.len(), before, "the shown rows are the old fold");
    let state = settle(state);
    assert!(matches!(
        state.tree.row(0),
        Some(crate::state::tree::Row::Group {
            expanded: false,
            ..
        })
    ));
}

#[test]
fn toggling_tree_mode_twice_during_a_job_flips_the_target_each_time() {
    let (state, _) = press(loaded(100, false), 't');
    assert!(state.rebuild_running());
    assert!(state.target_tree_mode());
    assert!(!state.tree_mode, "the shown list is still flat");
    let (state, _) = press(state, 't');
    assert!(!state.target_tree_mode());
    let state = settle(state);
    assert!(!state.tree_mode);
    let want = {
        let mut twin = sync_twin(&state);
        twin.rebuild_list();
        twin
    };
    assert_eq!(state.list, want.list);
}

#[test]
fn sort_is_inert_while_a_job_is_entering_tree_mode() {
    let (state, _) = press(loaded(100, false), 't');
    let (state, _) = press(state, 's');
    assert_eq!(state.list.sort, SortBy::Name);
    assert!(state.target_tree_mode());
}

/// Pages between steps, then the scan ending: the list ends where the
/// synchronous path does.
#[test]
fn scan_pages_between_steps_end_where_a_synchronous_rebuild_does() {
    let mut rng = Rng(201);
    let mut jobs = 0;
    for case in 0..150 {
        let tree = rng.below(2) == 0;
        let base = random_state(&mut rng, tree);
        let mut state = State {
            rebuild_slice: Some(1 + rng.below(40)),
            ..base
        };
        let mut twin = sync_twin(&state);
        let trigger = pick(&mut rng, &twin);
        crate::state::rebuild::tests::apply(&mut twin, &trigger, false);
        crate::state::rebuild::tests::apply(&mut state, &trigger, true);
        jobs += usize::from(state.rebuild_running());
        for _ in 0..(1 + rng.below(4)) {
            // A few steps, then a page.
            for _ in 0..rng.below(4) {
                (state, _) = update(state, Msg::RebuildStep);
            }
            let page: Vec<Vec<u8>> = (0..(1 + rng.below(60)))
                .map(|_| format!("{}:late", random_name(&mut rng)).into_bytes())
                .collect();
            (state, _) = update(state, Msg::ScanBatch { keys: page.clone() });
            (twin, _) = update(twin, Msg::ScanBatch { keys: page });
        }
        (state, _) = update(state, Msg::ScanComplete);
        (twin, _) = update(twin, Msg::ScanComplete);
        let state = settle(state);
        let what = format!("case {case} {trigger:?}");
        assert_same_shown(&state, &twin, &what);
    }
    assert!(jobs > 50, "only {jobs} jobs ran");
}

/// A job replaced mid-stage by another trigger ends where the two triggers
/// done in sequence, synchronously, do.
#[test]
fn a_job_replaced_mid_stage_ends_where_the_triggers_in_sequence_do() {
    let mut rng = Rng(203);
    let mut replaced = 0;
    for case in 0..300 {
        let tree = rng.below(2) == 0;
        let mut state = random_state(&mut rng, tree);
        let mut twin = sync_twin(&state);
        let first = pick(&mut rng, &twin);
        crate::state::rebuild::tests::apply(&mut twin, &first, false);
        crate::state::rebuild::tests::apply(&mut state, &first, true);
        for _ in 0..rng.below(30) {
            (state, _) = update(state, Msg::RebuildStep);
        }
        replaced += usize::from(state.rebuild_running());
        let second = pick(&mut rng, &twin);
        crate::state::rebuild::tests::apply(&mut twin, &second, false);
        crate::state::rebuild::tests::apply(&mut state, &second, true);
        let state = settle(state);
        let what = format!("case {case} {first:?} then {second:?}");
        assert_same_shown(&state, &twin, &what);
    }
    assert!(replaced > 100, "only {replaced} jobs were mid-flight");
}

/// A collapse during a job: the job's fold sees the toggle.
#[test]
fn a_collapse_during_a_full_job_ends_where_a_synchronous_collapse_does() {
    let mut rng = Rng(205);
    for case in 0..200 {
        let mut state = random_state(&mut rng, true);
        let mut twin = sync_twin(&state);
        let redo = crate::state::rebuild::tests::Trigger::Redo;
        crate::state::rebuild::tests::apply(&mut twin, &redo, false);
        crate::state::rebuild::tests::apply(&mut state, &redo, true);
        for _ in 0..rng.below(10) {
            (state, _) = update(state, Msg::RebuildStep);
        }
        let Some(prefix) = crate::state::rebuild::tests::random_prefix(&mut rng, &twin) else {
            continue;
        };
        let t = crate::state::rebuild::tests::Trigger::Toggle(prefix);
        crate::state::rebuild::tests::apply(&mut twin, &t, false);
        crate::state::rebuild::tests::apply(&mut state, &t, true);
        let state = settle(state);
        assert_same_shown(&state, &twin, &format!("case {case}"));
    }
}

fn assert_same_shown(got: &State, want: &State, what: &str) {
    assert_eq!(got.list, want.list, "{what}: list");
    assert_eq!(got.tree_mode, want.tree_mode, "{what}: tree_mode");
    // Out of tree mode the tree is stale by design (nothing reads it).
    if want.tree_mode {
        assert_eq!(got.tree, want.tree, "{what}: tree");
    }
}

#[test]
fn assert_same_is_reexported_for_the_engine_tests() {
    // Keeps the shared helper import honest.
    let mut rng = Rng(1);
    let state = random_state(&mut rng, false);
    assert_same(&state, &state.clone(), "identity");
}
