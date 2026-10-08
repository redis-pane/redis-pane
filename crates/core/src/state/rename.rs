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

/// Which key-level verb the one-line name capture is serving. The capture's
/// typing, validation and staging are shared; this decides the prefill, the
/// title and the staged mutation (M2 task 12,
/// `docs/plans/m2-task12-copy.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameKind {
    /// `R`: `RENAMENX`.
    Rename,
    /// `D`: `COPY` (or `DUMP` + `RESTORE` across slots on a Cluster).
    Copy,
    /// `R` in the value pane on a Hash field (M2 task 14,
    /// `docs/plans/m2-task14-member-rename.md`).
    Field,
    /// `R` in the value pane on a Set member.
    SetMember,
    /// `R` in the value pane on a ZSet member.
    ZSetMember,
}

impl NameKind {
    /// The capture's title, as drawn on its border and in the help overlay.
    pub fn title(&self) -> &'static str {
        match self {
            NameKind::Rename => "rename",
            NameKind::Copy => "duplicate key (COPY)",
            NameKind::Field => "rename field",
            NameKind::SetMember | NameKind::ZSetMember => "rename member",
        }
    }

    /// What the notices say before the colon: `rename: select a key first`.
    pub fn verb(&self) -> &'static str {
        match self {
            NameKind::Rename => "rename",
            NameKind::Copy => "duplicate",
            NameKind::Field => "rename field",
            NameKind::SetMember | NameKind::ZSetMember => "rename member",
        }
    }

    /// The key that starts it again, for "the key list changed" notices.
    pub fn key_label(&self) -> char {
        match self {
            NameKind::Rename | NameKind::Field | NameKind::SetMember | NameKind::ZSetMember => 'R',
            NameKind::Copy => 'D',
        }
    }

    /// Whether this renames something inside the Open key rather than a key.
    pub fn is_member(&self) -> bool {
        matches!(
            self,
            NameKind::Field | NameKind::SetMember | NameKind::ZSetMember
        )
    }

    /// What the value is called in "already in this hash".
    fn container(&self) -> &'static str {
        match self {
            NameKind::Field => "hash",
            NameKind::SetMember => "set",
            _ => "zset",
        }
    }
}

/// How a staged key-level command relates the two names' Cluster slots.
/// Computed once at staging, on a Cluster only; anywhere else it is
/// [`SlotPath::SameSlot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotPath {
    /// Standalone, or both names hash to one slot: the plain command runs.
    SameSlot,
    /// Different slots, and the command cannot cross them (`RENAME` fails
    /// with `CROSSSLOT`): `y` does nothing and only `Esc` leaves.
    CrossSlotRefused,
    /// Different slots, and the command has a fallback: a copy runs as
    /// `DUMP` + `PTTL` + `RESTORE` behind the same confirmation.
    CrossSlotFallback,
}

/// Why the typed name cannot be staged, shown under the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameProblem {
    Empty,
    Unchanged,
    /// Another field or member in the shown window already has the typed name
    /// (task 14). A name taken outside the window is the script's to refuse.
    Shown,
}

impl RenameProblem {
    pub fn reason(&self, kind: NameKind) -> &'static str {
        match (self, kind) {
            (RenameProblem::Empty, _) => "name can't be empty",
            (RenameProblem::Unchanged, NameKind::Rename) => "same as the current name",
            (RenameProblem::Unchanged, NameKind::Copy) => "same as the source name",
            (RenameProblem::Unchanged, _) => "same as the current name",
            (RenameProblem::Shown, k) => match k.container() {
                "hash" => "already a field in this hash",
                "set" => "already a member of this set",
                _ => "already a member of this zset",
            },
        }
    }
}

/// What a duplicate's name starts with after the source name.
pub const COPY_SUFFIX: &str = ":copy";

/// `R` or `D` is capturing a new name for the key at `index`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenameCapture {
    pub kind: NameKind,
    /// The Loaded set row the rename was started from, carried through to the
    /// staged mutation the way `DeleteKey` carries one.
    pub index: usize,
    pub from: KeyName,
    /// The Open key a field or member belongs to (task 14); `None` for a
    /// key-level rename or copy. For a field or member, `from` is the old
    /// name's bytes and `index` is the value cursor row it was started from.
    pub owner: Option<KeyName>,
    /// What has been typed. Starts as the current name (plus `:copy` for a
    /// duplicate); the cursor is always at the end.
    pub text: String,
}

impl RenameCapture {
    /// A capture prefilled with the current name. `None` for a name that is
    /// not valid UTF-8: no capture accepts byte escapes, so a binary name is
    /// refused rather than lossily rewritten (ADR-0017's rule for List edit).
    pub fn new(kind: NameKind, index: usize, from: KeyName) -> Option<Self> {
        let mut text = from.as_str()?.to_string();
        if kind == NameKind::Copy {
            text.push_str(COPY_SUFFIX);
        }
        Some(RenameCapture {
            kind,
            index,
            from,
            owner: None,
            text,
        })
    }

    /// A capture prefilled with a field's or member's current name, for the
    /// Open key `owner` (task 14). `None` for a name that is not valid UTF-8.
    pub fn member(kind: NameKind, owner: KeyName, row: usize, from: &[u8]) -> Option<Self> {
        let mut capture = RenameCapture::new(kind, row, KeyName::from(from.to_vec()))?;
        capture.owner = Some(owner);
        Some(capture)
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
