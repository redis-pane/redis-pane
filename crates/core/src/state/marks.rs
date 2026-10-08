//! Multi-select marks and the bulk-delete types (M2 task 13,
//! `docs/plans/m2-task13-bulk-delete.md`).
//!
//! Marks are a bitset over Loaded-set indices: toggle and membership are O(1)
//! and flat, and the memory is `Loaded / 8` bytes at worst (measured against a
//! sorted `Vec<u32>` in the plan's build design). It is lazily grown to the
//! highest index marked, so a session that never marks pays nothing. Nothing
//! here is per key: no struct, no name, only a bit.

/// How many key names the bulk-delete dialog lists before `… and N more`.
pub const BULK_PREVIEW_NAMES: usize = 8;

/// Which Loaded-set rows are marked for a bulk operation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Marks {
    words: Vec<u64>,
    count: usize,
}

impl Marks {
    /// How many rows are marked.
    pub fn count(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn is_marked(&self, i: usize) -> bool {
        self.words
            .get(i / 64)
            .is_some_and(|w| (w >> (i % 64)) & 1 == 1)
    }

    /// Flip row `i`. Returns whether it is marked afterwards.
    pub fn toggle(&mut self, i: usize) -> bool {
        let (word, bit) = (i / 64, 1u64 << (i % 64));
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        if self.words[word] & bit != 0 {
            self.words[word] &= !bit;
            self.count -= 1;
            false
        } else {
            self.words[word] |= bit;
            self.count += 1;
            true
        }
    }

    /// Unmark row `i`, if it was marked.
    pub fn unmark(&mut self, i: usize) {
        if let Some(w) = self.words.get_mut(i / 64) {
            let bit = 1u64 << (i % 64);
            if *w & bit != 0 {
                *w &= !bit;
                self.count -= 1;
            }
        }
    }

    /// Unmark everything and release the memory.
    pub fn clear(&mut self) {
        self.words = Vec::new();
        self.count = 0;
    }

    /// The marked indices in ascending order: O(Loaded / 64 + marked).
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(w, &bits)| {
            let mut rest = bits;
            std::iter::from_fn(move || {
                if rest == 0 {
                    return None;
                }
                let bit = rest.trailing_zeros() as usize;
                rest &= rest - 1;
                Some(w * 64 + bit)
            })
        })
    }

    /// Bytes held by the bitset.
    pub fn heap_bytes(&self) -> usize {
        self.words.capacity() * 8
    }
}

/// The `prod` typed-count gate on a staged bulk delete (DESIGN §6.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CountGate {
    /// Not a `prod` Connection: one `y` confirms.
    Open,
    /// `prod`: the dialog is up and `y` has not been pressed yet.
    Required,
    /// `prod`: the reader is typing the count. `wrong` is set by an `Enter`
    /// that did not match, and cleared by the next edit.
    Typing { text: String, wrong: bool },
}

/// A bulk delete that has been sent and not yet settled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkDelete {
    /// The Loaded-set row of each key, in the order the keys were sent.
    pub indices: Vec<u32>,
    /// Keys sent.
    pub total: usize,
    /// Keys the shell has finished batches for.
    pub done: usize,
    /// `Esc` was pressed and the shell has been told to stop.
    pub cancelling: bool,
    /// The numbering `indices` belong to.
    pub epoch: crate::command::MetadataEpoch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_counts_and_reverts() {
        let mut m = Marks::default();
        assert!(m.toggle(5));
        assert!(m.toggle(130));
        assert_eq!(m.count(), 2);
        assert!(m.is_marked(5) && m.is_marked(130) && !m.is_marked(6));
        assert!(!m.toggle(5));
        assert_eq!(m.count(), 1);
        assert!(!m.is_marked(5));
    }

    #[test]
    fn iteration_is_ascending() {
        let mut m = Marks::default();
        for i in [200, 3, 64, 63, 0, 1000] {
            m.toggle(i);
        }
        assert_eq!(m.iter().collect::<Vec<_>>(), vec![0, 3, 63, 64, 200, 1000]);
    }

    #[test]
    fn unmark_and_clear() {
        let mut m = Marks::default();
        m.toggle(7);
        m.toggle(9);
        m.unmark(7);
        m.unmark(7);
        m.unmark(100_000);
        assert_eq!(m.count(), 1);
        m.clear();
        assert!(m.is_empty());
        assert_eq!(m.heap_bytes(), 0);
        assert_eq!(m.iter().count(), 0);
    }

    #[test]
    fn nothing_is_allocated_until_something_is_marked() {
        assert_eq!(Marks::default().heap_bytes(), 0);
        assert!(!Marks::default().is_marked(1_000_000));
    }
}
