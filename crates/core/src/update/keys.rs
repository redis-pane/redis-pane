//! The keys pane: selection, the filter capture mode, tree folding, sort,
//! and opening a key from the list into the Viewer.

use super::*;

/// The row of the parent of `row` — the nearest *preceding* row one depth
/// shallower — or `None` at depth 0, which has no parent.
///
/// True by construction: `Tree::rebuild` pushes rows in a single
/// left-to-right pass and only ever reuses an already-pushed ancestor rather
/// than duplicating it, so the nearest preceding row at each shallower depth
/// is always the immediate parent. Works the same for a Key row or a Group
/// row — `Row::depth` is defined for both — which is what lets `Left`
/// (`Action::CollapseGroup`) treat "collapse, then go to parent" as one walk
/// regardless of what row it started on.
pub(super) fn parent_row(state: &State, row: usize) -> Option<usize> {
    use crate::state::tree::Row;
    let depth = state.tree.row(row)?.depth();
    if depth == 0 {
        return None;
    }
    (0..row)
        .rev()
        .find(|&r| matches!(state.tree.row(r), Some(Row::Group { depth: d, .. }) if d == depth - 1))
}

/// The prefix a group row stands for, e.g. `user:profile:` for a depth-1
/// group under `user:`.
///
/// Built from the rows themselves — this row and each ancestor Group row
/// above it via [`parent_row`], every one of which already carries its own
/// segment's arena location — so it needs nothing *beneath* `row` to exist.
/// An earlier version reconstructed the prefix by searching forward for the
/// first visible Key row under the group instead, which broke the moment the
/// group was collapsed: `Tree::row` hides a collapsed group's children
/// entirely, so the search skipped straight past them onto a *different*
/// group's key (or found nothing), reconstructing the wrong prefix or none
/// at all — folding a group worked, unfolding it from the same row didn't
/// (#16).
pub(super) fn group_prefix_at(state: &State, row: usize) -> Option<String> {
    use crate::state::tree::Row;
    let Some(Row::Group { .. }) = state.tree.row(row) else {
        return None;
    };

    let mut rows = vec![row];
    let mut r = row;
    while let Some(parent) = parent_row(state, r) {
        rows.push(parent);
        r = parent;
    }
    rows.reverse();

    let sep = state.tree.separator;
    let mut out = String::new();
    for r in rows {
        let Some(Row::Group { offset, len, .. }) = state.tree.row(r) else {
            return None;
        };
        out.push_str(&String::from_utf8_lossy(
            state.keys.arena_slice(offset, len)?,
        ));
        out.push(sep);
    }
    Some(out)
}

/// Move the selection, clamped to the Loaded set.
pub(super) fn move_selection(mut state: State, by: isize) -> (State, Vec<Command>) {
    if state.row_count() == 0 {
        return (state, Vec::new());
    }
    let last = state.row_count() as isize - 1;
    let next = (state.view.selected as isize + by).clamp(0, last);
    state.view.selected = next as usize;
    after_move(state)
}

/// Scrolling reveals rows whose metadata has not been fetched, so every move
/// asks for what is newly visible — and only what is visible (R2.4).
pub(super) fn after_move(mut state: State) -> (State, Vec<Command>) {
    // Moving in the key list is not scrolling the value, so the Viewer returns
    // to rest: whatever arrives next may land without asking.
    if let Some(open) = &mut state.open
        && open.offset == 0
    {
        open.at_rest = true;
    }
    let height = state.visible_rows();
    state.view = state.view.scrolled_to_selection(height);
    let indices = state.rows_needing_metadata();
    if indices.is_empty() {
        (state, Vec::new())
    } else {
        let epoch = state.metadata_epoch;
        (state, vec![Command::FetchMetadata { indices, epoch }])
    }
}

/// Keys typed while the filter is capturing.
///
/// Screen-routed: the Monitor view (`docs/plans/m3-monitor.md` decision 6)
/// reuses this same `/`-opens-filter-capture UX and the same
/// [`crate::state::view::matches`] predicate, but narrows its own
/// [`crate::state::MonitorState::filter`] rather than rebuilding the Loaded
/// set's `KeyView` — see `update::monitor::monitor_filter_key`.
pub(super) fn filter_key(state: State, key: KeyPress) -> (State, Vec<Command>) {
    if state.screen == crate::state::View::Monitor {
        return monitor_filter_key(state, key);
    }
    if state.screen == crate::state::View::PubSub {
        return pubsub_filter_key(state, key);
    }
    filter_key_keys_pane(state, key)
}

/// The filter text just changed (typed, deleted or pasted).
///
/// Two paths (M4 task 4, decisions 1 and 2). If the new text can only remove
/// rows from what is shown — [`KeyView::can_narrow`] — the list narrows from
/// its current rows immediately: cost is the size of the previous result, not
/// of the keyspace. Otherwise (backspace, a replacement, a lazily-sorted
/// view, keys not yet folded into the order) the full rebuild is *deferred*:
/// the text is already in the box, `filter_pending` records that the rows are
/// behind it, and the shell is asked to time a debounce window. Nothing else
/// changes on screen, so the input never lags; only the list catches up.
pub(super) fn filter_edited(mut state: State) -> (State, Vec<Command>) {
    if state.list.can_narrow(state.keys.len()) {
        state.rebuild_list_narrowing();
        after_move(state)
    } else {
        state.filter_pending = true;
        (state, vec![Command::ScheduleFilterRebuild])
    }
}

/// `Msg::FilterRebuildDue`: the debounce window closed. A no-op when nothing
/// is owed — an `Esc`, `Enter`, narrowing keystroke or scan rebuild may have
/// settled the list since the timer was armed.
pub(super) fn filter_rebuild_due(mut state: State) -> (State, Vec<Command>) {
    if !state.filter_pending {
        return (state, Vec::new());
    }
    state.rebuild_list();
    after_move(state)
}

fn filter_key_keys_pane(mut state: State, key: KeyPress) -> (State, Vec<Command>) {
    match key.code {
        KeyCode::Esc => {
            // Esc abandons the filter entirely rather than keeping a partial
            // pattern nobody typed on purpose.
            state.filtering = false;
            state.list.filter.clear();
            state.rebuild_list();
            after_move(state)
        }
        KeyCode::Enter => {
            state.filtering = false;
            // Leaving capture with rows still owed: settle now rather than
            // leave a filter on screen that the list does not obey.
            if state.filter_pending {
                state.rebuild_list();
                return after_move(state);
            }
            (state, Vec::new())
        }
        KeyCode::Backspace => {
            state.list.filter.pop();
            filter_edited(state)
        }
        KeyCode::Char(c) if !key.ctrl && !key.alt => {
            state.list.filter.push(c);
            filter_edited(state)
        }
        _ => (state, Vec::new()),
    }
}

/// `d` with the keys pane focused: stage a delete of the Selected key
/// (D4, PLAN M2 task 6).
///
/// The Selected key, not the Open key — the same target every other keys-pane
/// action takes. A gone row has nothing left to delete.
pub(super) fn delete_selected_key(mut state: State) -> (State, Vec<Command>) {
    let Some(index) = state.selected_key() else {
        return (state, Vec::new());
    };
    if state.keys.is_gone(index) {
        return (state, Vec::new());
    }
    let Some(name) = state.keys.name(index).map(KeyName::from) else {
        return (state, Vec::new());
    };
    state.confirm = Some(PendingMutation::DeleteKey { index, name });
    (state, Vec::new())
}

/// `→`/`Enter` on a row in the keys pane: expand a collapsed group, step into
/// an already-expanded one, or open a key.
pub(super) fn open_selected(mut state: State) -> (State, Vec<Command>) {
    // While the value cursor is active, Left/Right are not bound to
    // anything in the value pane (only Up/Down/PgUp/PgDn/Home/End
    // move the cursor there) — but the key list underneath is still
    // "selected" in the state-machine sense, so without this guard
    // Right/Left would silently walk the tree selection out from
    // under the open value. Esc, not an arrow key, is what leaves
    // cursor mode (`Action::Cancel`'s handler).
    if cursor_active(&state) {
        return (state, Vec::new());
    }
    // Right on a group row: expand it if collapsed, or step into its
    // first child if it is already expanded. Right never collapses —
    // `CollapseGroup` is the only key that does — matching the
    // standard treeview Right-arrow behavior (VS Code, macOS/Windows
    // outline views, the WAI-ARIA treeview pattern).
    if state.tree_mode
        && let Some(crate::state::tree::Row::Group { expanded, .. }) =
            state.tree.row(state.view.selected)
    {
        if expanded {
            return move_selection(state, 1);
        }
        if let Some(prefix) = group_prefix_at(&state, state.view.selected) {
            state.tree.toggle(&prefix);
            state.refold();
        }
        return after_move(state);
    }
    let Some(index) = state.selected_key() else {
        return (state, Vec::new());
    };
    let Some(name) = state.keys.name(index).map(KeyName::from) else {
        return (state, Vec::new());
    };
    // Opening a key moves focus onto it, at every width. Below 70
    // columns that is the "push" half of stack navigation (DESIGN §2);
    // above it both panes already show and this decides only which one
    // a pane-scoped key acts on. `Tab` and `Esc` both move it back
    // without closing the key — `Esc` additionally exits value-cursor
    // mode first, if it was active (`Action::Cancel`'s handler).
    state.focus = Pane::Value;
    let token = issue_read(&mut state);
    state.open_pending = Some(PendingRead {
        name: name.clone(),
        token,
        index: Some(index),
        issued_at_ms: None,
        activate_cursor: false,
        own_write: false,
    });
    let command = read_key(&state, name, Some(index), token);
    (state, vec![command])
}

/// `s`: cycle the active sort. A no-op in tree mode — folding needs name
/// order to group consecutive keys in one pass (`Tree::rebuild`'s doc
/// comment); cycling to another sort while folded fragments every group into
/// repeated headers with undercounted descendants. `rebuild_list` would snap
/// the sort back to Name immediately anyway, so this is a no-op either way —
/// guarding here just avoids advertising a key that visibly does nothing.
pub(super) fn cycle_sort(mut state: State) -> (State, Vec<Command>) {
    if state.tree_mode {
        return (state, Vec::new());
    }
    state.list.sort = state.list.sort.next();
    state.rebuild_list();
    after_move(state)
}

/// `t`: toggle tree mode.
pub(super) fn toggle_tree(mut state: State) -> (State, Vec<Command>) {
    state.tree_mode = !state.tree_mode;
    state.view.selected = 0;
    state.view.offset = 0;
    // Only the mode changed: tree mode keeps the flat view Name-sorted, so
    // entering it over a current Name view is a fold, and leaving it is no
    // work at all. Each falls back to a full rebuild when the view is not
    // current (M6 task 2).
    if state.tree_mode {
        state.refold();
    } else {
        state.leave_tree_mode();
    }
    after_move(state)
}

/// `←`: collapse an expanded group in place, or move the cursor to the
/// parent group (PLAN M2 task 6).
///
/// See the matching guard in [`open_selected`]. Left never expands (`Open`
/// is the only key that does): collapses an expanded group in place; a group
/// that is already collapsed, or a key row (which has no children of its own
/// to collapse), moves the cursor to the parent group instead. A top-level
/// group with nothing left to collapse has no parent to go to either, so
/// this is a no-op — the standard treeview Left-arrow behavior.
pub(super) fn collapse_group(mut state: State) -> (State, Vec<Command>) {
    if cursor_active(&state) {
        return (state, Vec::new());
    }
    if state.tree_mode {
        if let Some(crate::state::tree::Row::Group { expanded: true, .. }) =
            state.tree.row(state.view.selected)
        {
            if let Some(prefix) = group_prefix_at(&state, state.view.selected) {
                state.tree.toggle(&prefix);
                state.refold();
            }
        } else if let Some(parent) = parent_row(&state, state.view.selected) {
            state.view.selected = parent;
        }
    }
    after_move(state)
}

#[cfg(test)]
mod tree_fold_tests {
    //! `Enter` on a group row (`Action::ToggleGroup`) — fold and unfold must
    //! be the same operation from the same row (#16).

    use super::*;
    use crate::msg::KeyCode;
    use crate::state::tree::Row;

    /// Two levels deep, so folding and unfolding a nested group is exercised
    /// too, not just a top-level one.
    fn nested_tree() -> State {
        let mut state = State {
            cols: 130,
            rows: 30,
            tree_mode: true,
            ..State::default()
        };
        (state, _) = update(state, Msg::ScanStarted { estimated_total: 4 });
        let keys = vec![
            b"user:8812:cart".to_vec(),
            b"user:8812:session".to_vec(),
            b"user:8813:session".to_vec(),
            b"feed:global:hot".to_vec(),
        ];
        let (state, _) = update(state, Msg::ScanBatch { keys });
        state
    }

    fn press_left(state: State) -> State {
        update(state, Msg::Key(KeyPress::plain(KeyCode::Left))).0
    }

    fn press_right(state: State) -> State {
        update(state, Msg::Key(KeyPress::plain(KeyCode::Right))).0
    }

    fn group_row(state: &State, prefix: &str) -> usize {
        (0..state.tree.len())
            .find(|&r| {
                matches!(state.tree.row(r), Some(Row::Group { .. }))
                    && group_prefix_at(state, r).as_deref() == Some(prefix)
            })
            .unwrap_or_else(|| panic!("no group row for {prefix:?}"))
    }

    fn expanded(state: &State, row: usize) -> bool {
        matches!(state.tree.row(row), Some(Row::Group { expanded: true, .. }))
    }

    /// Pressing Sort in tree mode used to cycle `list.sort` to Ttl/Size/Kind —
    /// `rebuild_list` only ever snapped `Scan` back to `Name`, so any other
    /// sort stuck. `Tree::rebuild` requires a name-ordered view to fold same-
    /// prefix keys into one group in a single pass; under any other order it
    /// loses the run and emits a fresh header each time adjacency breaks,
    /// each with only part of the real descendant count — indistinguishable
    /// from "expanding a group doesn't show its children".
    #[test]
    fn sort_is_inert_in_tree_mode_so_folding_never_sees_another_order() {
        let mut state = State {
            cols: 130,
            rows: 30,
            tree_mode: true,
            ..State::default()
        };
        (state, _) = update(state, Msg::ScanStarted { estimated_total: 4 });
        let keys = vec![
            b"feed:a".to_vec(),
            b"user:1:x".to_vec(),
            b"feed:b".to_vec(),
            b"user:1:y".to_vec(),
        ];
        (state, _) = update(state, Msg::ScanBatch { keys });
        let before = state.tree.clone();

        (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('s'))));

        assert_eq!(
            state.list.sort,
            crate::state::view::SortBy::Name,
            "Sort must not move the list off name order while folded"
        );
        assert_eq!(
            state.tree, before,
            "a no-op Sort must leave the fold exactly as it was"
        );
        let group_rows = (0..state.tree.len())
            .filter(|&r| matches!(state.tree.row(r), Some(Row::Group { .. })))
            .count();
        assert_eq!(
            group_rows, 3,
            "one header each for feed:, user:, user:1: — not a fresh one \
             per broken run of adjacency"
        );
    }

    /// The regression this module exists for (#16, then re-fought over which
    /// keys should drive it): collapsing and expanding the same row must be
    /// the same operation done twice, whichever keys it lives on.
    #[test]
    fn left_collapses_in_place_and_right_expands_it_again_from_the_same_row() {
        let state = nested_tree();
        let before = state.tree.len();

        let row = group_row(&state, "user:");
        let mut state = state;
        state.view.selected = row;

        let state = press_left(state);
        assert!(!expanded(&state, row), "Left collapses the group");
        assert_eq!(
            state.view.selected, row,
            "collapsing does not move the cursor"
        );
        assert!(state.tree.len() < before, "children are hidden once folded");

        let state = press_right(state);
        assert!(expanded(&state, row), "Right expands it again, in place");
        assert_eq!(state.view.selected, row);
        assert_eq!(
            state.tree.len(),
            before,
            "back to exactly the structure it started with"
        );
    }

    #[test]
    fn a_nested_group_collapses_and_expands_independently_of_its_parent() {
        let state = nested_tree();
        let before = state.tree.len();

        let row = group_row(&state, "user:8812:");
        let mut state = state;
        state.view.selected = row;

        let state = press_left(state);
        assert!(!expanded(&state, row));
        // The parent group and the sibling `user:8813:` group are untouched.
        assert!(expanded(&state, group_row(&state, "user:")));

        let state = press_right(state);
        assert!(
            expanded(&state, row),
            "expands from the same row, same as the top-level case"
        );
        assert_eq!(state.tree.len(), before);
    }

    /// The standard treeview Right-arrow behavior: it never re-collapses an
    /// already-open group, it steps into the first child instead. Only Left
    /// ever collapses.
    #[test]
    fn right_on_an_expanded_group_steps_into_its_first_child_rather_than_collapsing_it() {
        let state = nested_tree();
        let row = group_row(&state, "user:");
        let mut state = state;
        state.view.selected = row;

        let state = press_right(state);
        assert!(expanded(&state, row), "Right must not have collapsed it");
        assert_eq!(
            state.view.selected,
            row + 1,
            "the cursor stepped into the first child row"
        );
    }

    /// The symmetric standard behavior for Left: it never expands anything,
    /// it only collapses or moves toward the root.
    #[test]
    fn left_on_an_already_collapsed_group_moves_to_its_parent() {
        let mut state = nested_tree();
        let row = group_row(&state, "user:8812:");
        state.tree.toggle(&group_prefix_at(&state, row).unwrap());
        state.rebuild_list();
        // The row index may have shifted once `user:8812:`'s children hid.
        let row = group_row(&state, "user:8812:");
        state.view.selected = row;

        let state = press_left(state);
        assert!(
            !expanded(&state, group_row(&state, "user:8812:")),
            "an already-collapsed group must not have been touched"
        );
        assert_eq!(
            state.view.selected,
            group_row(&state, "user:"),
            "the cursor moved to the parent group"
        );
    }

    #[test]
    fn left_on_a_key_row_moves_to_its_parent_group() {
        let state = nested_tree();
        let key_row = (0..state.tree.len())
            .find(|&r| state.tree.key_index(r).is_some())
            .expect("at least one key row is visible");
        let mut state = state;
        state.view.selected = key_row;

        let state = press_left(state);
        assert_eq!(
            state.view.selected,
            parent_row(&state, key_row).expect("a key always has a parent group"),
            "a key has no children of its own to collapse, so Left goes straight to its parent"
        );
    }

    /// The standard behavior at the root: a top-level group has nothing left
    /// to collapse into and no parent to jump to, so Left is a no-op.
    #[test]
    fn left_on_a_collapsed_top_level_group_is_a_no_op() {
        let mut state = nested_tree();
        let row = group_row(&state, "user:");
        state.tree.toggle(&group_prefix_at(&state, row).unwrap());
        state.rebuild_list();
        let row = group_row(&state, "user:");
        state.view.selected = row;
        let before = state.clone();

        let state = press_left(state);
        assert_eq!(state.view.selected, row, "the cursor does not move");
        assert_eq!(
            state.tree, before.tree,
            "fold state is unchanged: there was nothing left to collapse"
        );
    }

    /// Right on a key row is unaffected by any of this — it still opens the
    /// key, tree mode or not.
    #[test]
    fn right_on_a_key_row_still_opens_it() {
        let state = nested_tree();
        let key_row = (0..state.tree.len())
            .find(|&r| state.tree.key_index(r).is_some())
            .expect("at least one key row is visible");
        let mut state = state;
        state.view.selected = key_row;

        let (_, cmds) = update(state, Msg::Key(KeyPress::plain(KeyCode::Right)));
        assert!(
            matches!(cmds.as_slice(), [Command::ReadKey { .. }]),
            "expected an open, got {cmds:?}"
        );
    }
}

#[cfg(test)]
mod filter_narrowing_tests {
    //! M4 task 4, decisions 1 and 2: a filter edit may narrow from the
    //! previous result or defer to a debounced full rebuild, and either way
    //! the list the reader ends up with must be exactly what a from-scratch
    //! rebuild of the same text gives.

    use super::*;
    use crate::msg::{KeyCode, KeyPress, Msg};
    use crate::state::{FilterMode, KeyView, SortBy};

    #[derive(Clone)]
    enum Op {
        Type(char),
        Back,
        Paste(&'static str),
        Esc,
        Enter,
        /// The shell's debounce timer fires.
        Due,
        /// A page of this many new keys arrives from the scan.
        Keys(usize),
    }

    fn name(i: usize) -> Vec<u8> {
        format!(
            "{}:{:03}:{}",
            ["user", "feed", "cache"][i % 3],
            i,
            ["a", "ab", "b"][i % 3]
        )
        .into_bytes()
    }

    fn started(tree_mode: bool, sort: SortBy, mode: FilterMode, n: usize) -> State {
        let mut state = State {
            rows: 30,
            tree_mode,
            list: KeyView::new("", mode, sort),
            ..State::default()
        };
        (state, _) = update(
            state,
            Msg::ScanBatch {
                keys: (0..n).map(name).collect(),
            },
        );
        state.rebuild_list();
        state.filtering = true;
        state
    }

    fn fresh(state: &State) -> State {
        let mut want = State {
            rows: 30,
            tree_mode: state.tree_mode,
            list: KeyView::new(state.list.filter.clone(), state.list.mode, state.list.sort),
            ..State::default()
        };
        for i in 0..state.keys.len() {
            want.keys.push(state.keys.name(i).unwrap());
            if let Some(size) = state.keys.size(i) {
                want.keys.set_size(i, size);
            }
        }
        want.rebuild_list();
        want
    }

    fn rows(state: &State) -> Vec<Option<usize>> {
        (0..state.row_count()).map(|r| state.key_at(r)).collect()
    }

    fn run(mut state: State, ops: &[Op], case: &str) -> State {
        let mut total = state.keys.len();
        for op in ops {
            let msg = match op {
                Op::Type(c) => Msg::Key(KeyPress::plain(KeyCode::Char(*c))),
                Op::Back => Msg::Key(KeyPress::plain(KeyCode::Backspace)),
                Op::Paste(t) => Msg::Paste(t.to_string()),
                Op::Esc => Msg::Key(KeyPress::plain(KeyCode::Esc)),
                Op::Enter => Msg::Key(KeyPress::plain(KeyCode::Enter)),
                Op::Due => Msg::FilterRebuildDue,
                Op::Keys(n) => {
                    let keys = (total..total + n).map(name).collect();
                    total += n;
                    Msg::ScanBatch { keys }
                }
            };
            let was_filtering = state.filtering;
            (state, _) = update(state, msg);
            if matches!(op, Op::Due) || (was_filtering && matches!(op, Op::Esc | Op::Enter)) {
                assert!(!state.filter_pending, "{case}: settled, nothing owed");
            }
            // Whenever nothing is owed the rows must already be exactly what
            // a rebuild gives — and when something is owed the *text* must
            // already be current (checked by the final comparison below).
            if !state.filter_pending
                && !(matches!(op, Op::Keys(_))
                    && (state.tree_mode || state.list.sort != SortBy::Scan))
            {
                let want = fresh(&state);
                assert_eq!(rows(&state), rows(&want), "{case}: rows after an op");
            }
        }
        // The shell's timer always fires eventually.
        let (state, _) = update(state, Msg::FilterRebuildDue);
        let want = fresh(&state);
        assert_eq!(rows(&state), rows(&want), "{case}: settled rows");
        assert_eq!(state.tree, want.tree, "{case}: tree");
        assert_eq!(state.list.len(), want.list.len(), "{case}: match count");
        state
    }

    fn sequences() -> Vec<(&'static str, Vec<Op>)> {
        use Op::*;
        vec![
            ("first char", vec![Type('u')]),
            (
                "extending",
                vec![Type('u'), Type('s'), Type('e'), Type('r')],
            ),
            ("backspace", vec![Type('u'), Type('s'), Back, Type('e')]),
            ("backspace to empty", vec![Type('u'), Back, Back]),
            ("paste replacement", vec![Type('u'), Paste("ache:0"), Back]),
            ("paste on empty", vec![Paste("feed")]),
            ("clear with esc", vec![Type('u'), Type('s'), Esc]),
            ("enter flushes", vec![Type('u'), Type('s'), Back, Enter]),
            (
                "keys between keystrokes",
                vec![
                    Type('u'),
                    Keys(40),
                    Type('s'),
                    Keys(25),
                    Back,
                    Keys(10),
                    Type('e'),
                ],
            ),
            (
                "keys while a rebuild is pending",
                vec![Type('u'), Type('s'), Back, Keys(30), Type('s')],
            ),
            (
                "extension while pending is not narrowed from stale rows",
                vec![Type('f'), Type('e'), Back, Type('x'), Type('a'), Due],
            ),
            (
                "wildcard anchored at the end",
                vec![Type('*'), Type('a'), Type('b'), Back],
            ),
            (
                "wildcard glob extends",
                vec![Type('u'), Type('*'), Type('a'), Type('b')],
            ),
            ("due with nothing owed", vec![Due, Type('u'), Due, Due]),
            (
                "esc with pending",
                vec![Type('u'), Type('s'), Back, Esc, Due],
            ),
        ]
    }

    #[test]
    fn every_sequence_ends_where_a_full_rebuild_does_flat() {
        for mode in [FilterMode::Glob, FilterMode::Fuzzy] {
            for sort in [SortBy::Scan, SortBy::Name] {
                for (case, ops) in sequences() {
                    let label = format!("{case} / {mode:?} / {sort:?} / flat");
                    run(started(false, sort, mode, 60), &ops, &label);
                }
            }
        }
    }

    #[test]
    fn every_sequence_ends_where_a_full_rebuild_does_in_tree_mode() {
        for mode in [FilterMode::Glob, FilterMode::Fuzzy] {
            for (case, ops) in sequences() {
                let label = format!("{case} / {mode:?} / tree");
                run(started(true, SortBy::Name, mode, 60), &ops, &label);
            }
        }
    }

    #[test]
    fn every_sequence_ends_where_a_full_rebuild_does_under_a_lazy_sort() {
        for (case, ops) in sequences() {
            let mut state = started(false, SortBy::Size, FilterMode::Glob, 60);
            for i in (0..60).step_by(2) {
                state.keys.set_size(i, (i * 7 % 11) as u32);
            }
            state.rebuild_list();
            run(state, &ops, &format!("{case} / size sort"));
        }
    }

    #[test]
    fn an_extension_narrows_immediately_and_a_backspace_defers() {
        let state = started(false, SortBy::Scan, FilterMode::Glob, 60);
        let (state, cmds) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('u'))));
        assert!(
            cmds.iter()
                .all(|c| !matches!(c, Command::ScheduleFilterRebuild))
        );
        assert!(!state.filter_pending);
        assert_eq!(state.list.len(), 20, "narrowed at once");

        let (state, cmds) = update(state, Msg::Key(KeyPress::plain(KeyCode::Backspace)));
        assert!(cmds.contains(&Command::ScheduleFilterRebuild));
        assert!(state.filter_pending);
        assert_eq!(state.list.filter, "", "the box is current immediately");
        assert_eq!(state.list.len(), 20, "only the rows lag");

        let (state, _) = update(state, Msg::FilterRebuildDue);
        assert!(!state.filter_pending);
        assert_eq!(state.list.len(), 60);
    }

    #[test]
    fn esc_while_pending_clears_at_once_and_the_late_timer_is_a_no_op() {
        let state = started(false, SortBy::Scan, FilterMode::Glob, 60);
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('u'))));
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('s'))));
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Backspace)));
        assert!(state.filter_pending);
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Esc)));
        assert!(!state.filter_pending && !state.filtering);
        assert_eq!(state.list.len(), 60);
        let before = state.clone();
        let (after, cmds) = update(state, Msg::FilterRebuildDue);
        assert_eq!(after, before);
        assert!(cmds.is_empty());
    }

    #[test]
    fn a_scan_page_while_pending_does_not_mix_two_filters() {
        // "us" applied, Backspace deferred ("u" typed, rows still "us"), a
        // page arrives: new keys must be judged by the *applied* filter so
        // the order stays internally consistent until the rebuild.
        let state = started(false, SortBy::Scan, FilterMode::Glob, 60);
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('u'))));
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Char('s'))));
        let (state, _) = update(state, Msg::Key(KeyPress::plain(KeyCode::Backspace)));
        assert!(state.filter_pending);
        let keys = (60..90).map(name).collect();
        let (state, _) = update(state, Msg::ScanBatch { keys });
        for r in 0..state.list.len() {
            let n = state
                .keys
                .name_str(state.list.index_at(r).unwrap())
                .unwrap();
            assert!(
                n.to_lowercase().contains("us"),
                "{n} was judged by the wrong filter"
            );
        }
        let (state, _) = update(state, Msg::FilterRebuildDue);
        assert_eq!(rows(&state), rows(&fresh(&state)));
    }
}
