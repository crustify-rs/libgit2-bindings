//! Safe wrappers over `include/git2/tree.h`.
//!
//! `git_tree_walk` takes its traversal order (`git_treewalk_mode`) by value and refuses
//! any value the header does not name with a `GIT_ERROR_INVALID` error before reading
//! the tree, so a raw value is converted fallibly ([`WalkMode::from_raw`]) and never
//! transmuted.

use core::fmt;

use crate::ffi;

/// The order in which `git_tree_walk` calls back for a tree's entries and those of the
/// subtrees it loads on the way: either way, once per entry it reaches, each tree's entries in
/// that tree's order, under the path of the tree that holds them.
///
/// Wraps: git_treewalk_mode
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum WalkMode {
    /// Visit each entry before the subtree it names: a subtree's entries follow its
    /// own, and a positive callback result skips the subtree.
    PreOrder = ffi::GIT_TREEWALK_PRE,
    /// Visit each entry after the subtree it names: a subtree's entries precede its
    /// own, so the subtree is already walked, and a positive callback result is
    /// ignored.
    PostOrder = ffi::GIT_TREEWALK_POST,
}

impl WalkMode {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name, which `git_tree_walk` refuses.
    pub const fn from_raw(raw: ffi::git_treewalk_mode) -> Option<Self> {
        match raw {
            ffi::GIT_TREEWALK_PRE => Some(Self::PreOrder),
            ffi::GIT_TREEWALK_POST => Some(Self::PostOrder),
            _ => None,
        }
    }

    /// The C value, as `git_tree_walk` takes it.
    pub const fn raw(self) -> ffi::git_treewalk_mode {
        self as ffi::git_treewalk_mode
    }
}

impl TryFrom<ffi::git_treewalk_mode> for WalkMode {
    type Error = UnnamedWalkMode;

    fn try_from(raw: ffi::git_treewalk_mode) -> Result<Self, UnnamedWalkMode> {
        Self::from_raw(raw).ok_or(UnnamedWalkMode(raw))
    }
}

impl From<WalkMode> for ffi::git_treewalk_mode {
    fn from(value: WalkMode) -> ffi::git_treewalk_mode {
        value.raw()
    }
}

/// A tree walk mode that this version of libgit2's header does not name, which
/// `git_tree_walk` refuses as an invalid walking mode; [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedWalkMode(ffi::git_treewalk_mode);

impl UnnamedWalkMode {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_treewalk_mode {
        self.0
    }
}

impl fmt::Display for UnnamedWalkMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 tree walk mode {} is not named by git2/tree.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedWalkMode {}

#[cfg(test)]
mod unit_tests {
    use super::*;

    const MODES: [WalkMode; 2] = [WalkMode::PreOrder, WalkMode::PostOrder];

    #[test]
    fn raw_round_trips() {
        for mode in MODES {
            assert_eq!(WalkMode::from_raw(mode.raw()), Some(mode));
            assert_eq!(WalkMode::try_from(mode.raw()), Ok(mode));
            assert_eq!(ffi::git_treewalk_mode::from(mode), mode.raw());
        }
        assert_eq!(WalkMode::PreOrder.raw(), ffi::GIT_TREEWALK_PRE);
        assert_eq!(WalkMode::PostOrder.raw(), ffi::GIT_TREEWALK_POST);
    }

    #[test]
    fn unnamed_values_are_refused_without_panicking() {
        for raw in [2, 3, 0x100, ffi::git_treewalk_mode::MAX] {
            assert_eq!(WalkMode::from_raw(raw), None);
            let err = WalkMode::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 tree walk mode {raw} is not named by git2/tree.h")
            );
        }
    }

    #[test]
    fn layout_is_the_c_enum() {
        assert_eq!(size_of::<WalkMode>(), size_of::<ffi::git_treewalk_mode>());
        assert_eq!(size_of::<Option<WalkMode>>(), size_of::<WalkMode>());
    }
}
