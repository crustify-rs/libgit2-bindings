//! The branch kinds of `src/libgit2/branch.c` (`git_branch_t`, declared in
//! `include/git2/types.h`): [`BranchType`], the kinds a branch listing selects, and
//! [`UnnamedBranchType`], the error for a value the header does not name.

use core::fmt;
use core::hash::Hash;
use core::ops::BitOr;

use crate::ffi;

/// Which branches a branch routine means: local ones (under `refs/heads/`), remote-tracking
/// ones (under `refs/remotes/`), or both.
///
/// The C enum is flag-shaped, `GIT_BRANCH_ALL` being `GIT_BRANCH_LOCAL | GIT_BRANCH_REMOTE`,
/// and libgit2 reads it both ways: `git_branch_iterator_new` tests each bit, so `0` lists
/// nothing and other bits are ignored, while `git_branch_lookup` accepts the three named
/// values only and fails an internal assertion on any other (an error in release builds, an
/// abort in debug builds). `git_branch_next` reports a branch as [`Local`](Self::Local) or
/// [`Remote`](Self::Remote), never [`All`](Self::All). The three variants are the values
/// every routine accepts; [`from_raw`](Self::from_raw) gives `None` for the rest.
///
/// Wraps: `git_branch_t`
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum BranchType {
    /// Local branches, `refs/heads/*`.
    Local = ffi::GIT_BRANCH_LOCAL,
    /// Remote-tracking branches, `refs/remotes/*`.
    Remote = ffi::GIT_BRANCH_REMOTE,
    /// Both kinds.
    All = ffi::GIT_BRANCH_ALL,
}

impl BranchType {
    /// The variant whose C value is `raw`; `None` for any value libgit2's header does not
    /// name, `0` included.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_branch_t) -> Option<Self> {
        match raw {
            ffi::GIT_BRANCH_LOCAL => Some(Self::Local),
            ffi::GIT_BRANCH_REMOTE => Some(Self::Remote),
            ffi::GIT_BRANCH_ALL => Some(Self::All),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_branch_t {
        self as ffi::git_branch_t
    }

    /// Whether every branch `other` means is also one `self` means, as
    /// `git_branch_next` tests a listed branch's kind against the iterator's flags:
    /// [`All`](Self::All) contains every kind, and each kind contains itself.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.raw() & other.raw() == other.raw()
    }
}

/// The union of two selections: `Local | Remote` is [`All`](BranchType::All).
impl BitOr for BranchType {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        match (self, rhs) {
            (Self::Local, Self::Local) => Self::Local,
            (Self::Remote, Self::Remote) => Self::Remote,
            _ => Self::All,
        }
    }
}

impl TryFrom<ffi::git_branch_t> for BranchType {
    type Error = UnnamedBranchType;

    fn try_from(raw: ffi::git_branch_t) -> Result<Self, UnnamedBranchType> {
        Self::from_raw(raw).ok_or(UnnamedBranchType(raw))
    }
}

impl From<BranchType> for ffi::git_branch_t {
    fn from(value: BranchType) -> ffi::git_branch_t {
        value.raw()
    }
}

/// A branch type that libgit2's header does not name: `0`, which a branch iterator reads
/// as listing nothing, or a value with bits beyond `GIT_BRANCH_ALL`. [`raw`](Self::raw)
/// has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedBranchType(ffi::git_branch_t);

impl UnnamedBranchType {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_branch_t {
        self.0
    }
}

impl fmt::Display for UnnamedBranchType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unnamed branch type {}", self.0)
    }
}

impl std::error::Error for UnnamedBranchType {}
