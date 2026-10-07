//! The rename capture and its pre-check (M2 task 11,
//! `docs/plans/m2-task11-rename.md`).
//!
//! The capture is a modal single-line input in the shape of the keys-pane
//! filter: characters are text, `⌫` deletes, `Enter` stages, `Esc` discards.
//! It lives on `State`, not on the Open key like the Hash/Set add forms and
//! the TTL capture do, because the key being renamed is the Selected key and
//! need not be open.

use crate::key::KeyName;

/// What the `EXISTS` pre-check has said about a staged rename's target.
///
/// Advice only: `y` is never blocked by [`TargetCheck::Taken`], because
/// `RENAMENX` refuses a taken target atomically and is the real guard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetCheck {
    /// No answer yet (or the check failed, which is raised as its own error).
    Unchecked,
    /// `EXISTS` said no key has the new name.
    Free,
    /// `EXISTS` said a key already has the new name.
    Taken,
}

/// Why the typed name cannot be staged, shown under the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameProblem {
    Empty,
    Unchanged,
}

impl RenameProblem {
    pub fn reason(&self) -> &'static str {
        match self {
            RenameProblem::Empty => "name can't be empty",
            RenameProblem::Unchanged => "same as the current name",
        }
    }
}

/// `R` is capturing a new name for the key at `index`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameCapture {
    /// The Loaded set row the rename was started from, carried through to the
    /// staged mutation the way `DeleteKey` carries one.
    pub index: usize,
    pub from: KeyName,
    /// What has been typed. Starts as the current name; the cursor is always
    /// at the end.
    pub text: String,
}

impl RenameCapture {
    /// A capture prefilled with the current name. `None` for a name that is
    /// not valid UTF-8: no capture accepts byte escapes, so a binary name is
    /// refused rather than lossily rewritten (ADR-0017's rule for List edit).
    pub fn new(index: usize, from: KeyName) -> Option<Self> {
        let text = from.as_str()?.to_string();
        Some(RenameCapture { index, from, text })
    }

    pub fn problem(&self) -> Option<RenameProblem> {
        if self.text.is_empty() {
            Some(RenameProblem::Empty)
        } else if self.text.as_bytes() == self.from.as_bytes() {
            Some(RenameProblem::Unchanged)
        } else {
            None
        }
    }
}
