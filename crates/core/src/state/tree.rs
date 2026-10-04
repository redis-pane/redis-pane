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
        match std::str::from_utf8(prefix) {
            Ok(s) => self.collapsed.contains(s),
            Err(_) => self.collapsed.contains(&*String::from_utf8_lossy(prefix)),
        }
    }

    /// Rebuild the folded rows from the current view.
    ///
    /// Runs in one pass over the view's order, comparing each key's prefix
    /// segments against the previous key's. That is only correct on a
    /// name-ordered view, which is why tree mode sorts by name.
    pub fn rebuild(&mut self, keys: &LoadedSet, view: &KeyView) {
        self.rows.clear();
        let sep = self.separator as u8;
        // Two segment buffers for the whole pass, swapped per key: the
        // rebuild allocates no per-key `Vec` (M4 task 4, decision 3).
        let mut previous: Vec<(u32, u16)> = Vec::new();
        let mut segments: Vec<(u32, u16)> = Vec::new();
        // Row indices of the currently-open ancestor Group rows, kept in
        // step with `previous` (truncated on divergence, extended on a new
        // group) rather than recomputed from `self.rows` afterwards — that
        // is what lets a collapsed group's count include children that
        // never become rows of their own. `counts` is parallel to
        // `self.rows`, filled in here and applied once at the end.
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

            split_segments(name, sep, start, &mut segments);
            // How many leading segments this key shares with the previous one.
            //
            // Compared by *bytes*, not by `(offset, len)`: the same prefix text
            // sits at a different arena offset in every key that carries it, so
            // comparing handles would find nothing in common and emit a fresh
            // group header for every single key.
            let shared = segments
                .iter()
                .zip(previous.iter())
                .take_while(|((ao, al), (bo, bl))| {
                    keys.arena_slice(*ao, *al) == keys.arena_slice(*bo, *bl)
                })
                .count();
            open_groups.truncate(shared);

            // `shared` is a *byte* comparison against `previous`'s full
            // segment list, which is set below regardless of whether the
            // matching key actually got rows for every one of those
            // segments. If it broke early because an ancestor was collapsed,
            // `previous` still carries every segment past that point — so
            // the next key in a *different* branch under the same collapsed
            // ancestor (e.g. `cache:user:…` right after `cache:page:…`, both
            // hidden under a collapsed `cache:`) can still show `shared >= 1`
            // purely because their leading bytes coincide, and `depth <
            // shared` below would then `continue` straight past re-checking
            // whether `cache:` is collapsed — walking right into creating a
            // fresh `user:` row underneath a parent marked shut (#reported
            // as "collapsing cache: leaves one child visible").
            //
            // `open_groups` does not have this problem: it only ever holds
            // rows that were actually pushed, so if an ancestor was
            // collapsed, its row is always the *last* entry (nothing deeper
            // was ever opened past a `break`). Checking it directly is the
            // fix — anything still open and collapsed hides this key's
            // entire remaining subtree, group rows included.
            let hidden_by_collapsed_ancestor = open_groups.last().is_some_and(|&r| {
                matches!(
                    self.rows[r],
                    Row::Group {
                        expanded: false,
                        ..
                    }
                )
            });

            if !hidden_by_collapsed_ancestor {
                for (depth, (offset, len)) in segments.iter().enumerate() {
                    if depth < shared {
                        continue;
                    }
                    // The prefix through this segment and its separator is a
                    // slice of the key's own name — segments are contiguous.
                    let end = (*offset - start) as usize + *len as usize + 1;
                    let collapsed = self.prefix_is_collapsed(&name[..end]);
                    self.rows.push(Row::Group {
                        offset: *offset,
                        len: *len,
                        depth: depth as u16,
                        descendants: 0,
                        expanded: !collapsed,
                    });
                    counts.push(0);
                    open_groups.push(self.rows.len() - 1);
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

            // Every ancestor still open at this key's depth gains one
            // descendant — the group itself if it is collapsed, same as an
            // expanded one; folding hides rows, not the count of what they
            // represent.
            for &g in &open_groups {
                counts[g] += 1;
            }

            // If any ancestor is collapsed, the key itself is not shown.
            if !self.ancestor_collapsed(name) {
                self.rows.push(Row::Key {
                    index: index as u32,
                    depth: segments.len() as u16,
                });
                // `counts` is indexed the same as `self.rows`, so every push
                // to one needs a push to the other — this entry is never
                // read back (only `Row::Group` rows are), it just keeps the
                // two in step.
                counts.push(0);
            }
            std::mem::swap(&mut previous, &mut segments);
        }

        self.inverse.clear();
        self.inverse.resize(keys.len(), NO_ROW);
        for (i, row) in self.rows.iter_mut().enumerate() {
            match row {
                Row::Group { descendants, .. } => *descendants = counts[i],
                Row::Key { index, .. } => {
                    if let Some(slot) = self.inverse.get_mut(*index as usize) {
                        *slot = i as u32;
                    }
                }
            }
        }
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

/// Split a key name into arena-relative `(offset, len)` segments.
///
/// The trailing segment (the leaf) is excluded: it is the key itself, not a
/// group. `a:b:c` yields the groups `a` and `b`.
///
/// Writes into a caller-owned buffer (cleared first) so the per-key loop in
/// [`Tree::rebuild`] reuses two allocations for the whole pass.
fn split_segments(name: &[u8], sep: u8, arena_start: u32, out: &mut Vec<(u32, u16)>) {
    out.clear();
    let mut begin = 0usize;
    for (i, b) in name.iter().enumerate() {
        if *b == sep {
            out.push((arena_start + begin as u32, (i - begin) as u16));
            begin = i + 1;
        }
    }
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
}
