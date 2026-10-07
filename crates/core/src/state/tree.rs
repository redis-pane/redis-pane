//! Tree mode: folding the key list on a separator (R2.3, PLAN M1.6).
//!
//! **The tree holds no key names.** Every node points into the same byte arena
//! the [`LoadedSet`](super::LoadedSet) already owns, as an `(offset, len)` pair
//! borrowed from whichever key first produced that prefix. A second copy of a
//! million key names would cost more than the entire rest of the structure
//! (ADR-0010).

use std::collections::HashSet;

use super::loaded::LoadedSet;
use super::view::KeyView;

/// "No row" in an inverse index (a Loaded set index that is not shown).
pub(super) const NO_ROW: u32 = u32::MAX;

/// What a rendered row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A folded prefix, e.g. `user:` — with how many keys sit beneath it.
    Group {
        /// Where the *segment* text lives in the arena.
        offset: u32,
        len: u16,
        depth: u16,
        descendants: u32,
        expanded: bool,
    },
    /// An actual key. `index` is its position in the Loaded set.
    Key { index: u32, depth: u16 },
}

impl Row {
    pub fn depth(&self) -> u16 {
        match self {
            Row::Group { depth, .. } | Row::Key { depth, .. } => *depth,
        }
    }
}

/// The folded view of the current [`KeyView`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    rows: Vec<Row>,
    /// Prefixes the user has collapsed, stored as full prefix strings. Small by
    /// construction: only what someone has actually clicked shut. A set, so a
    /// membership test costs one hash of the probe rather than a walk of every
    /// collapsed prefix (M4 task 4, decision 4). Keyed by the `String` the
    /// caller toggled — see [`Tree::prefix_is_collapsed`] for why not by
    /// arena `(offset, len)`.
    collapsed: HashSet<String>,
    /// The distinct byte lengths of the members of `collapsed`. A prefix of
    /// any other length cannot be one of them, so the fold skips the hash
    /// (a SipHash of the whole prefix) for the overwhelming majority of
    /// group rows (M6 task 5).
    collapsed_lens: Vec<usize>,
    /// How many members of `collapsed` do *not* end in the separator. The
    /// O(1) ancestor test probes only separator-terminated prefixes of a
    /// key's name; a hand-toggled prefix that does not end in one (nothing in
    /// the app produces that, but `toggle` takes any `&str`) would be missed,
    /// so while any exists the old linear `starts_with` scan is used instead.
    irregular: usize,
    /// Inverse of `rows`' `Key` entries: Loaded set index -> row, or
    /// [`NO_ROW`]. Dense and sized to the Loaded set at the last
    /// [`Tree::rebuild`] (ADR-0010: parallel arrays, no per-key struct).
    inverse: Vec<u32>,
    pub separator: char,
}

/// Where a fold has got to: [`Tree::rebuild`]'s loop state as a value, so the
/// rebuild job can stop after a slice and resume on the next step.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct FoldProgress {
    /// The next view row to place.
    next: usize,
    /// Where the separators sit in the last key placed, as byte positions in
    /// its name. The next key keeps the ones inside the prefix it shares
    /// (M6 task 5): one buffer for the whole fold, no per-key `Vec`.
    seps: Vec<u16>,
    /// The last key placed, as an arena `(start, len)`: the fallback's
    /// comparison target when the view carries no LCP.
    prev: (u32, u16),
    /// Keys placed so far (hidden ones too).
    placed: u32,
    /// The currently-open ancestor Group rows: row index, and `placed` when
    /// the group opened. A group's descendant count is `placed` at the
    /// moment it closes minus `placed` at the moment it opened, so no
    /// per-key pass touches every open group.
    open_groups: Vec<(usize, u32)>,
    /// `Some(cursor)` once every key is placed: the next row to fix up.
    fixup: Option<usize>,
}

impl FoldProgress {
    /// Heap bytes the loop state holds.
    pub(super) fn heap_bytes(&self) -> usize {
        self.open_groups.capacity() * std::mem::size_of::<(usize, u32)>()
            + self.seps.capacity() * std::mem::size_of::<u16>()
    }
}

impl Default for Tree {
    fn default() -> Self {
        // `:` is the near-universal Redis convention; it is configurable
        // because "near-universal" is not "always".
        Tree::new(':')
    }
}

impl Tree {
    pub fn new(separator: char) -> Self {
        Self {
            rows: Vec::new(),
            collapsed: HashSet::new(),
            collapsed_lens: Vec::new(),
            irregular: 0,
            inverse: Vec::new(),
            separator,
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn row(&self, i: usize) -> Option<Row> {
        self.rows.get(i).copied()
    }

    /// Heap bytes held by the folded rows, the inverse index and the
    /// collapsed-prefix set.
    /// `Row` carries no key names — every node borrows `(offset, len)` into
    /// the same arena the `LoadedSet` already owns — so this is just the row
    /// vector plus whatever a reader has actually clicked shut. Used
    /// alongside `LoadedSet::heap_bytes` and `KeyView::heap_bytes` by the M4
    /// memory-budget test.
    pub fn heap_bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<Row>()
            + self.inverse.capacity() * std::mem::size_of::<u32>()
            // Hash table slot (String + one control byte), approximately.
            + self.collapsed.capacity() * (std::mem::size_of::<String>() + 1)
            + self.collapsed.iter().map(|s| s.capacity()).sum::<usize>()
    }

    /// The Loaded set index at this row, if the row is a key rather than a group.
    pub fn key_index(&self, row: usize) -> Option<usize> {
        match self.rows.get(row) {
            Some(Row::Key { index, .. }) => Some(*index as usize),
            _ => None,
        }
    }

    /// The display row a Loaded set index is folded at, if it has a row of
    /// its own (a key under a collapsed group has none). O(1).
    pub fn row_of(&self, index: usize) -> Option<usize> {
        match self.inverse.get(index) {
            Some(&r) if r != NO_ROW => Some(r as usize),
            _ => None,
        }
    }

    pub fn toggle(&mut self, prefix: &str) {
        if self.collapsed.remove(prefix) {
            if !prefix.as_bytes().ends_with(&[self.separator as u8]) {
                self.irregular -= 1;
            }
        } else {
            if !prefix.as_bytes().ends_with(&[self.separator as u8]) {
                self.irregular += 1;
            }
            self.collapsed.insert(prefix.to_string());
        }
        self.collapsed_lens = self.collapsed.iter().map(String::len).collect();
        self.collapsed_lens.sort_unstable();
        self.collapsed_lens.dedup();
    }

    pub fn is_collapsed(&self, prefix: &str) -> bool {
        self.collapsed.contains(prefix)
    }

    /// `is_collapsed` for a prefix still in arena bytes. Valid UTF-8 (every
    /// real key) probes the set with a borrowed `&str` — no allocation. Only
    /// a prefix with invalid bytes pays for the lossy conversion the old
    /// per-segment `String` always did, so a collapsed `\u{FFFD}:` group
    /// still matches exactly what it matched before. Keyed by `String`
    /// rather than arena `(offset, len)`: an offset names *one key's* copy of
    /// the text, so the same prefix would need a byte comparison per probe
    /// anyway, and the set is a handful of entries, not a column.
    fn prefix_is_collapsed(&self, prefix: &[u8]) -> bool {
        if self.collapsed.is_empty() {
            return false;
        }
        match std::str::from_utf8(prefix) {
            Ok(s) => self.collapsed_lens.contains(&s.len()) && self.collapsed.contains(s),
            Err(_) => self.collapsed.contains(&*String::from_utf8_lossy(prefix)),
        }
    }

    /// Rebuild the folded rows from the current view.
    ///
    /// Runs in one pass over the view's order, comparing each key's prefix
    /// segments against the previous key's. That is only correct on a
    /// name-ordered view, which is why tree mode sorts by name.
    ///
    /// The synchronous form of the sliced fold the rebuild job steps
    /// (`fold_begin` / `fold_step`, M6 task 3): one code path, so the two
    /// cannot disagree.
    pub fn rebuild(&mut self, keys: &LoadedSet, view: &KeyView) {
        let order = view.order();
        let lcp = view.name_lcp();
        let mut progress = self.fold_begin();
        while !self.fold_step(keys, order, lcp, keys.len(), &mut progress, usize::MAX) {}
    }

    /// A tree with this one's collapsed set and separator and no rows: what a
    /// rebuild job folds into while this one stays on screen.
    pub(super) fn shell(&self) -> Tree {
        Tree {
            rows: Vec::new(),
            collapsed: self.collapsed.clone(),
            collapsed_lens: self.collapsed_lens.clone(),
            irregular: self.irregular,
            inverse: Vec::new(),
            separator: self.separator,
        }
    }

    /// Start a fold: clears the rows and
    /// returns the loop state [`Tree::fold_step`] resumes.
    pub(super) fn fold_begin(&mut self) -> FoldProgress {
        // No `reserve`: the row count is unknown (groups add rows), and a
        // reserve of the key count makes the Vec's doubling land on a
        // capacity twice the size it otherwise would (64 MB, not 33, at 1M).
        self.rows.clear();
        FoldProgress::default()
    }

    /// How far a fold has got, 0.0 to 1.0: the key pass is most of the work,
    /// the descendant-count pass the rest.
    pub(super) fn fold_fraction(&self, p: &FoldProgress, order_len: usize) -> f32 {
        match p.fixup {
            None if order_len == 0 => 0.0,
            None => 0.85 * p.next as f32 / order_len as f32,
            Some(_) if self.rows.is_empty() => 1.0,
            Some(c) => 0.85 + 0.15 * c as f32 / self.rows.len() as f32,
        }
    }

    /// Advance a fold by at most `limit` view rows (then, once every key has
    /// been placed, `limit` rows of the fix-up pass that writes the inverse).
    /// Returns whether the fold is finished.
    ///
    /// `order` is the name-ordered view's index vector; `inverse_len` sizes
    /// the inverse (the Loaded set's length the view covers). `lcp`, when the
    /// view has one, is parallel to `order`: how many bytes each name shares
    /// with the one before it. Without it the fold measures the shared
    /// prefix itself, one byte comparison per key.
    pub(super) fn fold_step(
        &mut self,
        keys: &LoadedSet,
        order: &[u32],
        lcp: Option<&[u32]>,
        inverse_len: usize,
        p: &mut FoldProgress,
        limit: usize,
    ) -> bool {
        let sep = self.separator as u8;
        if p.fixup.is_none() {
            let end = p.next.saturating_add(limit).min(order.len());
            while p.next < end {
                let pos = p.next;
                let index = order[pos] as usize;
                p.next += 1;
                let Some(name) = keys.name(index) else {
                    continue;
                };
                let Some(start) = keys.name_offset(index) else {
                    continue;
                };
                let shared_bytes = match lcp.and_then(|l| l.get(pos)) {
                    Some(&l) => l as usize,
                    None => {
                        let before = keys.arena_slice(p.prev.0, p.prev.1).unwrap_or(&[]);
                        common_prefix(before, name)
                    }
                };
                self.fold_key(p, index, name, start, shared_bytes, sep);
            }
            if p.next < order.len() {
                return false;
            }
            // Every group still open closes with the last key.
            while let Some((row, opened)) = p.open_groups.pop() {
                if let Row::Group { descendants, .. } = &mut self.rows[row] {
                    *descendants = p.placed - opened;
                }
            }
            p.fixup = Some(0);
            self.inverse.clear();
            self.inverse.resize(inverse_len, NO_ROW);
            return false;
        }
        let cursor = p.fixup.unwrap_or(0);
        let end = cursor.saturating_add(limit).min(self.rows.len());
        for i in cursor..end {
            if let Row::Key { index, .. } = self.rows[i]
                && let Some(slot) = self.inverse.get_mut(index as usize)
            {
                *slot = i as u32;
            }
        }
        p.fixup = Some(end);
        end >= self.rows.len()
    }

    /// Place one key: the body of the fold loop. `shared_bytes` is how many
    /// leading bytes `name` shares with the previous key's name.
    fn fold_key(
        &mut self,
        p: &mut FoldProgress,
        index: usize,
        name: &[u8],
        start: u32,
        shared_bytes: usize,
        sep: u8,
    ) {
        let lcp = shared_bytes.min(name.len());
        // How many leading segments this key shares with the previous one.
        //
        // A segment is shared exactly when its terminating separator lies
        // inside the shared prefix: both names then carry the same bytes
        // through that separator, so the segments up to it are equal; and a
        // separator at or past the LCP ends a segment whose bytes differ
        // (or whose end is not in the other name). That is the old
        // segment-by-segment byte comparison, answered from the previous
        // key's separator positions alone. The same prefix text sits at a
        // different arena offset in every key that carries it, which is why
        // this is a *byte* question and never a comparison of `(offset,
        // len)` handles.
        let shared = p.seps.iter().take_while(|&&s| (s as usize) < lcp).count();
        p.seps.truncate(shared);
        // The shared prefix holds exactly those separators and no others, so
        // only the bytes from the LCP on need looking at.
        for (i, &b) in name[lcp..].iter().enumerate() {
            if b == sep {
                p.seps.push((lcp + i) as u16);
            }
        }
        p.prev = (start, name.len() as u16);

        // Groups that closed with the previous key: their count is final.
        while p.open_groups.len() > shared {
            if let Some((row, opened)) = p.open_groups.pop()
                && let Row::Group { descendants, .. } = &mut self.rows[row]
            {
                *descendants = p.placed - opened;
            }
        }

        // `shared` says how many segments the *previous key's bytes* have in
        // common with this one, whether or not the previous key actually got
        // rows for every one of them. If it broke early because an ancestor
        // was collapsed, `p.seps` still carries every separator past that
        // point — so the next key in a *different* branch under the same
        // collapsed ancestor (e.g. `cache:user:…` right after
        // `cache:page:…`, both hidden under a collapsed `cache:`) can still
        // show `shared >= 1` purely because their leading bytes coincide,
        // and skipping straight to `depth < shared` would walk past
        // re-checking whether `cache:` is collapsed and create a fresh
        // `user:` row underneath a parent marked shut (#reported as
        // "collapsing cache: leaves one child visible").
        //
        // `open_groups` does not have this problem: it only ever holds
        // rows that were actually pushed, so if an ancestor was
        // collapsed, its row is always the *last* entry (nothing deeper
        // was ever opened past a `break`). Checking it directly is the
        // fix — anything still open and collapsed hides this key's
        // entire remaining subtree, group rows included.
        let hidden_by_collapsed_ancestor = self.last_open_is_collapsed(p);

        if !hidden_by_collapsed_ancestor {
            for depth in shared..p.seps.len() {
                let begin = if depth == 0 {
                    0
                } else {
                    p.seps[depth - 1] as usize + 1
                };
                let at = p.seps[depth] as usize;
                // The prefix through this segment and its separator is a
                // slice of the key's own name — segments are contiguous.
                let collapsed = self.prefix_is_collapsed(&name[..=at]);
                self.rows.push(Row::Group {
                    offset: start + begin as u32,
                    len: (at - begin) as u16,
                    depth: depth as u16,
                    descendants: 0,
                    expanded: !collapsed,
                });
                p.open_groups.push((self.rows.len() - 1, p.placed));
                if collapsed {
                    // Stop descending: every key under this prefix is
                    // hidden. The collapsed group's own row stays in
                    // `open_groups`, though — its count must keep
                    // growing even though none of these keys get a row
                    // of their own.
                    break;
                }
            }
        }

        // If any ancestor is collapsed, the key itself is not shown. The
        // groups above stop at the first collapsed prefix, so that prefix
        // is the last open group exactly when one exists — the same answer
        // as probing every prefix of the name, without the probes. A
        // collapsed prefix that does not end in the separator is invisible
        // to the groups, so while one exists the old per-prefix test runs.
        let hidden = if self.collapsed.is_empty() {
            false
        } else if self.irregular > 0 {
            self.ancestor_collapsed(name)
        } else {
            self.last_open_is_collapsed(p)
        };
        if !hidden {
            self.rows.push(Row::Key {
                index: index as u32,
                depth: p.seps.len() as u16,
            });
        }
        p.placed += 1;
    }

    /// Whether the innermost open group is a collapsed one.
    fn last_open_is_collapsed(&self, p: &FoldProgress) -> bool {
        p.open_groups.last().is_some_and(|&(r, _)| {
            matches!(
                self.rows[r],
                Row::Group {
                    expanded: false,
                    ..
                }
            )
        })
    }

    /// Whether any collapsed prefix is a prefix of `name`. One set probe per
    /// separator in the name — O(separators), independent of how many groups
    /// are collapsed.
    fn ancestor_collapsed(&self, name: &[u8]) -> bool {
        if self.collapsed.is_empty() {
            return false;
        }
        if self.irregular > 0 {
            let name = String::from_utf8_lossy(name);
            return self.collapsed.iter().any(|p| name.starts_with(p.as_str()));
        }
        let sep = self.separator as u8;
        name.iter()
            .enumerate()
            .any(|(i, b)| *b == sep && self.prefix_is_collapsed(&name[..=i]))
    }
}

/// How many leading bytes `a` and `b` share.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    let m = a.len().min(b.len());
    let mut i = 0;
    while i + 8 <= m {
        let x = u64::from_le_bytes(a[i..i + 8].try_into().unwrap_or([0; 8]));
        let y = u64::from_le_bytes(b[i..i + 8].try_into().unwrap_or([0; 8]));
        if x != y {
            return i + ((x ^ y).trailing_zeros() / 8) as usize;
        }
        i += 8;
    }
    while i < m && a[i] == b[i] {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::view::SortBy;

    fn built(names: &[&str]) -> (LoadedSet, KeyView, Tree) {
        let mut keys = LoadedSet::default();
        for n in names {
            keys.push(n.as_bytes());
        }
        let mut view = KeyView::new("", crate::state::FilterMode::Glob, SortBy::Name);
        view.rebuild(&keys);
        let mut tree = Tree::new(':');
        tree.rebuild(&keys, &view);
        (keys, view, tree)
    }

    fn rendered(keys: &LoadedSet, tree: &Tree) -> Vec<String> {
        (0..tree.len())
            .map(|i| match tree.row(i).unwrap() {
                Row::Group {
                    offset,
                    len,
                    depth,
                    descendants,
                    ..
                } => format!(
                    "{}{}: ({descendants})",
                    "  ".repeat(depth as usize),
                    String::from_utf8_lossy(keys.arena_slice(offset, len).unwrap())
                ),
                Row::Key { index, depth } => format!(
                    "{}{}",
                    "  ".repeat(depth as usize),
                    keys.name_str(index as usize).unwrap()
                ),
            })
            .collect()
    }

    #[test]
    fn keys_fold_on_the_separator() {
        let (keys, _, tree) = built(&[
            "user:1:session",
            "user:1:cart",
            "user:2:session",
            "feed:hot",
        ]);
        assert_eq!(
            rendered(&keys, &tree),
            [
                "feed: (1)",
                "  feed:hot",
                "user: (3)",
                "  1: (2)",
                "    user:1:cart",
                "    user:1:session",
                "  2: (1)",
                "    user:2:session",
            ]
        );
    }

    #[test]
    fn collapsed_nodes_show_their_child_count_and_hide_their_children() {
        let (keys, view, mut tree) = built(&["user:1:a", "user:1:b", "feed:hot"]);
        tree.toggle("user:");
        tree.rebuild(&keys, &view);
        let rows = rendered(&keys, &tree);
        assert_eq!(
            rows,
            ["feed: (1)", "  feed:hot", "user: (2)"],
            "a collapsed group states what it is hiding, count included — \
             hidden children must not read as an empty group"
        );
    }

    /// A collapsed group's count must include *every* real descendant, not
    /// just the ones that happen to have a row of their own — a nested
    /// group collapsed two levels down used to be invisible to
    /// `count_descendants`, which only ever counted rows it could see.
    #[test]
    fn a_nested_collapse_still_counts_toward_every_open_ancestor() {
        let (keys, view, mut tree) = built(&[
            "user:1:cart",
            "user:1:session",
            "user:2:session",
            "feed:hot",
        ]);
        tree.toggle("user:1:");
        tree.rebuild(&keys, &view);
        assert_eq!(
            rendered(&keys, &tree),
            [
                "feed: (1)",
                "  feed:hot",
                "user: (3)",
                "  1: (2)",
                "  2: (1)",
                "    user:2:session",
            ],
            "user:'s count must still include the two keys hidden under the \
             collapsed user:1: subgroup"
        );
    }

    /// Collapsing a top-level group used to leave a *later* nested subgroup
    /// visible, still expanded, right where the collapsed group's children
    /// should be hidden entirely — reported as "collapsing cache: leaves
    /// user: showing". The cause: `shared` matches `cache:page:…` and
    /// `cache:user:…` only at depth 0 (byte comparison, not row identity),
    /// so once the `page:` keys had broken out of the loop under the
    /// collapsed `cache:`, the very next key — in the `user:` branch —
    /// walked straight past the `depth < shared` reuse check without ever
    /// re-examining whether `cache:` (its open ancestor) was collapsed.
    #[test]
    fn a_collapsed_top_level_group_hides_every_sibling_subgroup_not_just_the_first() {
        let (keys, view, mut tree) = built(&[
            "cache:page:aaa",
            "cache:page:bbb",
            "cache:user:aaa",
            "cache:user:bbb",
            "counter:hits",
        ]);
        tree.toggle("cache:");
        tree.rebuild(&keys, &view);
        assert_eq!(
            rendered(&keys, &tree),
            ["cache: (4)", "counter: (1)", "  counter:hits"],
            "no page: or user: row, however far into the collapsed group's \
             keys the fold walks — a sibling subgroup discovered after the \
             first one must stay just as hidden"
        );
    }

    #[test]
    fn a_key_with_no_separator_is_a_leaf_at_the_root() {
        let (keys, _, tree) = built(&["standalone"]);
        assert_eq!(rendered(&keys, &tree), ["standalone"]);
    }

    #[test]
    fn the_tree_stores_no_key_names_of_its_own() {
        // R2.3 and ADR-0010: a second copy of a million names would cost more
        // than the whole rest of the structure. Every group points into the
        // arena the store already owns.
        let (keys, _, tree) = built(&["user:1:session", "user:2:session"]);
        for i in 0..tree.len() {
            if let Some(Row::Group { offset, len, .. }) = tree.row(i) {
                assert!(
                    keys.arena_slice(offset, len).is_some(),
                    "a group pointed outside the arena"
                );
            }
        }
        assert_eq!(
            std::mem::size_of::<Row>(),
            16,
            "a row is a handful of integers, not a string"
        );
    }

    #[test]
    fn toggling_twice_returns_to_where_it_started() {
        let (keys, view, mut tree) = built(&["user:1:a", "user:1:b"]);
        let before = rendered(&keys, &tree);
        tree.toggle("user:");
        tree.rebuild(&keys, &view);
        tree.toggle("user:");
        tree.rebuild(&keys, &view);
        assert_eq!(rendered(&keys, &tree), before);
    }

    // ── M4 task 4: equivalence with the pre-change implementation ──────────

    /// The pre-task-4 `Tree::rebuild`, verbatim apart from its inputs: a
    /// `Vec<String>` of collapsed prefixes scanned linearly, a fresh segment
    /// `Vec` per key and a `String` prefix built per segment. Kept in the
    /// test module as the oracle the allocation-free, O(1)-lookup version
    /// must agree with row for row.
    fn reference_rows(
        collapsed: &[String],
        sep_char: char,
        keys: &LoadedSet,
        view: &KeyView,
    ) -> Vec<Row> {
        let is_collapsed = |p: &str| collapsed.iter().any(|c| c == p);
        let ancestor_collapsed = |name: &[u8]| {
            if collapsed.is_empty() {
                return false;
            }
            let name = String::from_utf8_lossy(name);
            collapsed.iter().any(|p| name.starts_with(p.as_str()))
        };
        let split = |name: &[u8], sep: u8, arena_start: u32| {
            let mut out = Vec::new();
            let mut begin = 0usize;
            for (i, b) in name.iter().enumerate() {
                if *b == sep {
                    out.push((arena_start + begin as u32, (i - begin) as u16));
                    begin = i + 1;
                }
            }
            out
        };
        let mut rows: Vec<Row> = Vec::new();
        let sep = sep_char as u8;
        let mut previous: Vec<(u32, u16)> = Vec::new();
        let mut open_groups: Vec<usize> = Vec::new();
        let mut counts: Vec<u32> = Vec::new();
        for row in 0..view.len() {
            let Some(index) = view.index_at(row) else {
                continue;
            };
            let Some(name) = keys.name(index) else {
                continue;
            };
            let Some(start) = keys.name_offset(index) else {
                continue;
            };
            let segments = split(name, sep, start);
            let shared = segments
                .iter()
                .zip(previous.iter())
                .take_while(|((ao, al), (bo, bl))| {
                    keys.arena_slice(*ao, *al) == keys.arena_slice(*bo, *bl)
                })
                .count();
            open_groups.truncate(shared);
            let hidden = open_groups.last().is_some_and(|&r| {
                matches!(
                    rows[r],
                    Row::Group {
                        expanded: false,
                        ..
                    }
                )
            });
            if !hidden {
                let mut prefix = String::new();
                for (depth, (offset, len)) in segments.iter().enumerate() {
                    prefix.push_str(&String::from_utf8_lossy(
                        keys.arena_slice(*offset, *len).unwrap_or_default(),
                    ));
                    prefix.push(sep_char);
                    if depth < shared {
                        continue;
                    }
                    let c = is_collapsed(&prefix);
                    rows.push(Row::Group {
                        offset: *offset,
                        len: *len,
                        depth: depth as u16,
                        descendants: 0,
                        expanded: !c,
                    });
                    counts.push(0);
                    open_groups.push(rows.len() - 1);
                    if c {
                        break;
                    }
                }
            }
            for &g in &open_groups {
                counts[g] += 1;
            }
            if !ancestor_collapsed(name) {
                rows.push(Row::Key {
                    index: index as u32,
                    depth: segments.len() as u16,
                });
                counts.push(0);
            }
            previous = segments;
        }
        for (i, row) in rows.iter_mut().enumerate() {
            if let Row::Group { descendants, .. } = row {
                *descendants = counts[i];
            }
        }
        rows
    }

    /// A deterministic mixed keyspace: 1-5 segments, empty segments, a
    /// trailing separator, keys with no separator, a prefix that is also a
    /// key, and names with invalid UTF-8.
    fn fixture_keys() -> LoadedSet {
        let mut keys = LoadedSet::default();
        let words = ["user", "feed", "cache", "a", "", "zz", "x1"];
        let mut state = 0x2545_f491_u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for _ in 0..400 {
            let depth = 1 + (next() % 5) as usize;
            let mut name = Vec::new();
            for d in 0..depth {
                if d > 0 {
                    name.push(b':');
                }
                name.extend_from_slice(words[(next() % words.len() as u32) as usize].as_bytes());
                name.extend_from_slice(format!("{}", next() % 3).as_bytes());
            }
            keys.push(&name);
        }
        for extra in [
            &b"user"[..],
            b"user0:",
            b"user0:1:",
            b"standalone",
            b"bad:\xff\xfe:leaf",
            b"bad:\xff\xfe:other:leaf",
            b"bad:\xfe\xff:leaf",
            b"caf\xc3\xa9:x:y",
        ] {
            keys.push(extra);
        }
        keys
    }

    fn collapsed_sets() -> Vec<Vec<&'static str>> {
        vec![
            vec![],
            vec!["user0:"],
            vec!["user0:", "feed1:", "cache2:"],
            vec!["user0:", "user0:a1:", "user0:a1:feed2:"],
            vec!["a0:", "a1:", "a2:", "zz0:", "zz1:", "x10:"],
            vec!["nothing:here:"],
            vec!["bad:\u{fffd}:"],
            vec!["bad:\u{fffd}:", "café:", "café:x:"],
            // Irregular: no trailing separator — matches by raw starts_with.
            vec!["user"],
            vec!["feed1", "cache2:"],
            vec!["user0:", "0:"],
            vec![":"],
        ]
    }

    #[test]
    fn rebuild_matches_the_reference_implementation_for_every_collapsed_set() {
        let keys = fixture_keys();
        let mut view = KeyView::new("", crate::state::FilterMode::Glob, SortBy::Name);
        view.rebuild(&keys);
        for set in collapsed_sets() {
            let mut tree = Tree::new(':');
            for p in &set {
                tree.toggle(p);
            }
            tree.rebuild(&keys, &view);
            let owned: Vec<String> = set.iter().map(|p| p.to_string()).collect();
            let want = reference_rows(&owned, ':', &keys, &view);
            assert_eq!(tree.rows, want, "rows differ for collapsed set {set:?}");
            // Rebuilding again over the same buffers changes nothing.
            tree.rebuild(&keys, &view);
            assert_eq!(tree.rows, want, "second rebuild differs for {set:?}");
        }
    }

    #[test]
    fn rebuild_matches_the_reference_on_a_filtered_view() {
        let keys = fixture_keys();
        let mut view = KeyView::new("user*", crate::state::FilterMode::Glob, SortBy::Name);
        view.rebuild(&keys);
        assert!(!view.is_empty());
        let mut tree = Tree::new(':');
        tree.toggle("user0:");
        tree.rebuild(&keys, &view);
        let want = reference_rows(&["user0:".to_string()], ':', &keys, &view);
        assert_eq!(tree.rows, want);
    }

    #[test]
    fn collapsed_lookups_answer_as_the_linear_scan_did() {
        let keys = fixture_keys();
        for set in collapsed_sets() {
            let mut tree = Tree::new(':');
            for p in &set {
                tree.toggle(p);
            }
            let linear_is = |probe: &str| set.contains(&probe);
            let linear_anc = |name: &[u8]| {
                let name = String::from_utf8_lossy(name);
                set.iter().any(|p| name.starts_with(p))
            };
            for i in 0..keys.len() {
                let name = keys.name(i).unwrap();
                assert_eq!(
                    tree.ancestor_collapsed(name),
                    linear_anc(name),
                    "ancestor_collapsed({:?}) with {set:?}",
                    String::from_utf8_lossy(name)
                );
                let probe = String::from_utf8_lossy(name).into_owned();
                assert_eq!(tree.is_collapsed(&probe), linear_is(&probe));
                let with_sep = format!("{probe}:");
                assert_eq!(tree.is_collapsed(&with_sep), linear_is(&with_sep));
            }
        }
    }

    #[test]
    fn toggle_twice_clears_irregular_bookkeeping() {
        let mut tree = Tree::new(':');
        tree.toggle("user");
        assert_eq!(tree.irregular, 1);
        tree.toggle("user");
        assert_eq!(tree.irregular, 0);
        assert!(!tree.is_collapsed("user"));
    }

    #[test]
    fn tree_row_of_matches_a_linear_scan_for_every_index() {
        let keys = fixture_keys();
        let mut view = KeyView::new("", crate::state::FilterMode::Glob, SortBy::Name);
        view.rebuild(&keys);
        for set in collapsed_sets() {
            let mut tree = Tree::new(':');
            for p in &set {
                tree.toggle(p);
            }
            tree.rebuild(&keys, &view);
            for index in 0..keys.len() + 3 {
                let linear = (0..tree.len()).find(|&r| tree.key_index(r) == Some(index));
                assert_eq!(tree.row_of(index), linear, "index {index}, set {set:?}");
            }
        }
    }

    // ── LCP-driven fold (M6 task 5, docs/plans/m6-fast-fold.md) ─────────────

    /// The fold exactly as it was before task 5: split every key into
    /// segments, compare them with the previous key's byte by byte, probe the
    /// collapsed set once per separator. Kept verbatim as the oracle the LCP
    /// fold must equal for every separator, including a multi-byte one
    /// (truncated to its low byte, as `Tree` always did).
    fn legacy_rows(tree: &Tree, keys: &LoadedSet, view: &KeyView) -> Vec<Row> {
        let sep = tree.separator as u8;
        let split = |name: &[u8], start: u32| {
            let mut out: Vec<(u32, u16)> = Vec::new();
            let mut begin = 0usize;
            for (i, b) in name.iter().enumerate() {
                if *b == sep {
                    out.push((start + begin as u32, (i - begin) as u16));
                    begin = i + 1;
                }
            }
            out
        };
        let mut rows: Vec<Row> = Vec::new();
        let mut counts: Vec<u32> = Vec::new();
        let mut open_groups: Vec<usize> = Vec::new();
        let mut previous: Vec<(u32, u16)> = Vec::new();
        for pos in 0..view.len() {
            let index = view.order()[pos] as usize;
            let (Some(name), Some(start)) = (keys.name(index), keys.name_offset(index)) else {
                continue;
            };
            let segments = split(name, start);
            let shared = segments
                .iter()
                .zip(previous.iter())
                .take_while(|((ao, al), (bo, bl))| {
                    keys.arena_slice(*ao, *al) == keys.arena_slice(*bo, *bl)
                })
                .count();
            open_groups.truncate(shared);
            let hidden = open_groups.last().is_some_and(|&r| {
                matches!(
                    rows[r],
                    Row::Group {
                        expanded: false,
                        ..
                    }
                )
            });
            if !hidden {
                for (depth, (offset, len)) in segments.iter().enumerate() {
                    if depth < shared {
                        continue;
                    }
                    let end = (*offset - start) as usize + *len as usize + 1;
                    let collapsed = tree.prefix_is_collapsed(&name[..end]);
                    rows.push(Row::Group {
                        offset: *offset,
                        len: *len,
                        depth: depth as u16,
                        descendants: 0,
                        expanded: !collapsed,
                    });
                    counts.push(0);
                    open_groups.push(rows.len() - 1);
                    if collapsed {
                        break;
                    }
                }
            }
            for &g in &open_groups {
                counts[g] += 1;
            }
            if !tree.ancestor_collapsed(name) {
                rows.push(Row::Key {
                    index: index as u32,
                    depth: segments.len() as u16,
                });
                counts.push(0);
            }
            previous = segments;
        }
        for (i, row) in rows.iter_mut().enumerate() {
            if let Row::Group { descendants, .. } = row {
                *descendants = counts[i];
            }
        }
        rows
    }

    /// Fold with an explicit LCP column (or none), slice by slice.
    fn fold_with(
        tree: &Tree,
        keys: &LoadedSet,
        view: &KeyView,
        lcp: Option<&[u32]>,
        slice: usize,
    ) -> Tree {
        let mut out = tree.shell();
        let mut p = out.fold_begin();
        while !out.fold_step(keys, view.order(), lcp, keys.len(), &mut p, slice) {}
        out
    }

    fn brute_lcp(keys: &LoadedSet, view: &KeyView) -> Vec<u32> {
        (0..view.len())
            .map(|r| {
                if r == 0 {
                    return 0;
                }
                let a = keys.name(view.order()[r - 1] as usize).unwrap();
                let b = keys.name(view.order()[r] as usize).unwrap();
                a.iter().zip(b).take_while(|(x, y)| x == y).count() as u32
            })
            .collect()
    }

    struct Xs(u64);
    impl Xs {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n as u64) as usize
        }
    }

    /// Names shaped to hit every boundary: empty segments, a trailing
    /// separator, no separator, a key that is a prefix of its neighbour,
    /// duplicate-prefix segments, and raw bytes (the separator's own low
    /// byte, and invalid UTF-8).
    fn random_names(rng: &mut Xs, sep: char, n: usize) -> LoadedSet {
        let sep_byte = sep as u32 as u8;
        let words: [&[u8]; 9] = [
            b"a", b"ab", b"abc", b"b", b"", b"cache", b"user", b"\xff", b"1",
        ];
        let mut keys = LoadedSet::default();
        for _ in 0..n {
            let mut name: Vec<u8> = Vec::new();
            let segs = rng.below(5);
            for s in 0..segs {
                if s > 0 {
                    name.push(sep_byte);
                }
                name.extend_from_slice(words[rng.below(words.len())]);
            }
            match rng.below(6) {
                0 => name.push(sep_byte),
                1 => name.extend_from_slice(b"zz"),
                _ => {}
            }
            keys.push(&name);
        }
        keys
    }

    fn prefixes_of(keys: &LoadedSet, sep: u8, rng: &mut Xs, count: usize) -> Vec<String> {
        let mut out = Vec::new();
        for _ in 0..count {
            let name = keys.name(rng.below(keys.len())).unwrap();
            let seps: Vec<usize> = (0..name.len()).filter(|&i| name[i] == sep).collect();
            if seps.is_empty() {
                continue;
            }
            let at = seps[rng.below(seps.len())];
            let mut prefix = String::from_utf8_lossy(&name[..=at]).into_owned();
            // An occasional prefix that does not end in the separator.
            if rng.below(8) == 0 {
                prefix.pop();
            }
            out.push(prefix);
        }
        out
    }

    #[test]
    fn the_lcp_fold_equals_the_legacy_fold_for_every_separator_and_collapsed_set() {
        let mut rng = Xs(0x9e37_79b9_7f4a_7c15);
        for case in 0..400 {
            let sep = [':', '/', '.', '\u{e9}', '\u{2192}'][case % 5];
            let n = 1 + rng.below(60);
            let keys = random_names(&mut rng, sep, n);
            let mut view = KeyView::new("", crate::state::FilterMode::Glob, SortBy::Name);
            view.rebuild(&keys);
            let lcp = view
                .name_lcp()
                .expect("a Name-sorted rebuild carries an LCP");
            assert_eq!(lcp, brute_lcp(&keys, &view), "case {case}: view lcp");
            for round in 0..4 {
                let mut tree = Tree::new(sep);
                if round > 0 {
                    for prefix in prefixes_of(&keys, sep as u32 as u8, &mut rng, round) {
                        tree.toggle(&prefix);
                    }
                }
                let want = legacy_rows(&tree, &keys, &view);
                for (what, got) in [
                    (
                        "view lcp",
                        fold_with(&tree, &keys, &view, Some(lcp), usize::MAX),
                    ),
                    (
                        "view lcp, slices",
                        fold_with(&tree, &keys, &view, Some(lcp), 3),
                    ),
                    ("no lcp", fold_with(&tree, &keys, &view, None, usize::MAX)),
                    ("no lcp, slices", fold_with(&tree, &keys, &view, None, 2)),
                ] {
                    assert_eq!(got.rows, want, "case {case}.{round} sep {sep:?}: {what}");
                    for index in 0..keys.len() {
                        let linear = want.iter().position(
                            |r| matches!(r, Row::Key { index: i, .. } if *i as usize == index),
                        );
                        assert_eq!(got.row_of(index), linear, "case {case}.{round}: inverse");
                    }
                }
            }
        }
    }

    /// The reported shape: with `cache:` shut, the next key under `cache:user:`
    /// shares leading bytes with the hidden `cache:page:` keys and must not
    /// grow a visible child.
    #[test]
    fn the_cache_user_regression_holds_with_an_lcp() {
        let (keys, view, mut tree) = built(&[
            "cache:page:1",
            "cache:page:2",
            "cache:user:1",
            "cache:user:2",
            "feed:x",
        ]);
        tree.toggle("cache:");
        let want = legacy_rows(&tree, &keys, &view);
        let got = fold_with(&tree, &keys, &view, view.name_lcp(), usize::MAX);
        assert_eq!(got.rows, want);
        assert_eq!(
            rendered(&keys, &got),
            ["cache: (4)", "feed: (1)", "  feed:x"]
        );
        tree.toggle("cache:");
        tree.toggle("cache:user:");
        let got = fold_with(&tree, &keys, &view, view.name_lcp(), usize::MAX);
        assert_eq!(got.rows, legacy_rows(&tree, &keys, &view));
        assert_eq!(
            rendered(&keys, &got),
            [
                "cache: (4)",
                "  page: (2)",
                "    cache:page:1",
                "    cache:page:2",
                "  user: (2)",
                "feed: (1)",
                "  feed:x"
            ]
        );
    }

    /// A boundary exactly at the LCP: `a:b` then `a:c` share `a:` (the
    /// separator is inside the prefix), `ab` then `a:` share only `a`.
    #[test]
    fn a_separator_at_the_lcp_boundary_counts_only_when_inside_the_shared_prefix() {
        for names in [
            vec!["a:b", "a:c"],
            vec!["a", "a:"],
            vec!["a:", "a:b"],
            vec!["a:b", "a:b:c"],
            vec!["a::b", "a::c", "a:b"],
            vec!["a:b:", "a:b:c"],
            vec!["", ":", "::", ":a"],
        ] {
            let (keys, view, tree) = built(&names);
            let got = fold_with(&tree, &keys, &view, view.name_lcp(), usize::MAX);
            assert_eq!(got.rows, legacy_rows(&tree, &keys, &view), "{names:?}");
        }
    }

    #[test]
    fn narrowing_keeps_the_lcp_column_exact() {
        let mut rng = Xs(77);
        for case in 0..200 {
            let n = 2 + rng.below(80);
            let keys = random_names(&mut rng, ':', n);
            let mut view = KeyView::new("", crate::state::FilterMode::Glob, SortBy::Name);
            view.rebuild(&keys);
            for text in ["a", "ab", "ab:"] {
                view.filter = text.to_string();
                if !view.can_narrow(keys.len()) {
                    continue;
                }
                view.narrow(&keys);
                assert_eq!(
                    view.name_lcp().expect("narrowed view keeps its LCP"),
                    brute_lcp(&keys, &view),
                    "case {case} filter {text:?}"
                );
            }
        }
    }

    #[test]
    fn only_a_name_sorted_order_carries_an_lcp() {
        let mut keys = LoadedSet::default();
        for n in ["b:1", "a:1", "a:2"] {
            keys.push(n.as_bytes());
        }
        let mut view = KeyView::new("", crate::state::FilterMode::Glob, SortBy::Scan);
        view.rebuild(&keys);
        assert!(view.name_lcp().is_none());
        view.sort = SortBy::Name;
        view.rebuild(&keys);
        assert_eq!(view.name_lcp(), Some(&[0, 2, 0][..]));
        keys.push(b"a:3");
        view.sort = SortBy::Scan;
        view.rebuild(&keys);
        keys.push(b"a:4");
        view.extend(&keys, 4);
        assert!(view.name_lcp().is_none());
    }
}
