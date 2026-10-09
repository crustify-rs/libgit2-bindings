//! Safe wrappers over `include/git2/types.h`.

use core::fmt;

use crate::ffi;

/// Whether `git fetch` recurses into a submodule: the value of its
/// `submodule.<name>.fetchRecurseSubmodules` setting.
///
/// libgit2 reads `false` as [`No`](Self::No), `true` as [`Yes`](Self::Yes) and
/// `on-demand` as [`OnDemand`](Self::OnDemand), and writes the latter two back that
/// way; it writes [`No`](Self::No), also what an unset setting reads as, by removing
/// the setting. `git_submodule_set_fetch_recurse_submodules` refuses any value
/// without a variant ("invalid value for fetchRecurseSubmodules"), and
/// [`from_raw`](Self::from_raw) gives `None` for it.
///
/// Wraps: git_submodule_recurse_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum SubmoduleRecurse {
    /// Do not recurse into the submodule.
    No = ffi::GIT_SUBMODULE_RECURSE_NO,
    /// Recurse into the submodule.
    Yes = ffi::GIT_SUBMODULE_RECURSE_YES,
    /// Recurse into the submodule only when the commit the superproject records is
    /// not already in its local clone.
    OnDemand = ffi::GIT_SUBMODULE_RECURSE_ONDEMAND,
}

impl SubmoduleRecurse {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_submodule_recurse_t) -> Option<Self> {
        match raw {
            ffi::GIT_SUBMODULE_RECURSE_NO => Some(Self::No),
            ffi::GIT_SUBMODULE_RECURSE_YES => Some(Self::Yes),
            ffi::GIT_SUBMODULE_RECURSE_ONDEMAND => Some(Self::OnDemand),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    pub const fn raw(self) -> ffi::git_submodule_recurse_t {
        self as ffi::git_submodule_recurse_t
    }
}

/// What libgit2 reports for a submodule whose configuration does not set
/// `fetchRecurseSubmodules`: [`No`](Self::No).
impl Default for SubmoduleRecurse {
    fn default() -> Self {
        Self::No
    }
}

impl TryFrom<ffi::git_submodule_recurse_t> for SubmoduleRecurse {
    type Error = UnnamedSubmoduleRecurse;

    fn try_from(raw: ffi::git_submodule_recurse_t) -> Result<Self, UnnamedSubmoduleRecurse> {
        Self::from_raw(raw).ok_or(UnnamedSubmoduleRecurse(raw))
    }
}

impl From<SubmoduleRecurse> for ffi::git_submodule_recurse_t {
    fn from(value: SubmoduleRecurse) -> ffi::git_submodule_recurse_t {
        value.raw()
    }
}

/// A submodule recursion setting that this version of libgit2's header does not
/// name, which libgit2 refuses to write. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedSubmoduleRecurse(ffi::git_submodule_recurse_t);

impl UnnamedSubmoduleRecurse {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_submodule_recurse_t {
        self.0
    }
}

impl fmt::Display for UnnamedSubmoduleRecurse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 submodule recurse setting {} is not named by git2/types.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedSubmoduleRecurse {}

/// How `git submodule update` updates a submodule: the value of its
/// `submodule.<name>.update` setting.
///
/// libgit2 reads `checkout`, `rebase`, `merge` and `none` as the variants of those
/// names, `true` as [`Checkout`](Self::Checkout) and `false` as [`None`](Self::None),
/// and writes each variant back by its name.
///
/// The header also names `GIT_SUBMODULE_UPDATE_DEFAULT` (0), "not used except as
/// static initializer": no libgit2 routine produces it,
/// `git_submodule_set_update` refuses it like any other value without a variant
/// ("invalid value for update"), and `git_submodule_update_strategy` would report it
/// as [`Checkout`](Self::Checkout). So it has no variant: [`from_raw`](Self::from_raw)
/// gives `None` for it, and `from_raw(raw).unwrap_or_default()` resolves it as
/// libgit2 does.
///
/// Wraps: git_submodule_update_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum SubmoduleUpdate {
    /// Check the new commit out as a detached `HEAD` of the submodule.
    Checkout = ffi::GIT_SUBMODULE_UPDATE_CHECKOUT,
    /// Rebase the submodule's current branch onto the new commit.
    Rebase = ffi::GIT_SUBMODULE_UPDATE_REBASE,
    /// Merge the new commit into the submodule's current branch.
    Merge = ffi::GIT_SUBMODULE_UPDATE_MERGE,
    /// Leave the submodule alone, even when the superproject's commit changes.
    None = ffi::GIT_SUBMODULE_UPDATE_NONE,
}

impl SubmoduleUpdate {
    /// The variant whose C value is `raw`; `None` for `GIT_SUBMODULE_UPDATE_DEFAULT`
    /// and for a value this version of libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_submodule_update_t) -> Option<Self> {
        match raw {
            ffi::GIT_SUBMODULE_UPDATE_CHECKOUT => Some(Self::Checkout),
            ffi::GIT_SUBMODULE_UPDATE_REBASE => Some(Self::Rebase),
            ffi::GIT_SUBMODULE_UPDATE_MERGE => Some(Self::Merge),
            ffi::GIT_SUBMODULE_UPDATE_NONE => Some(Self::None),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    pub const fn raw(self) -> ffi::git_submodule_update_t {
        self as ffi::git_submodule_update_t
    }
}

/// What libgit2 reports for a submodule whose configuration does not set `update`,
/// and what it resolves `GIT_SUBMODULE_UPDATE_DEFAULT` to:
/// [`Checkout`](Self::Checkout).
impl Default for SubmoduleUpdate {
    fn default() -> Self {
        Self::Checkout
    }
}

impl TryFrom<ffi::git_submodule_update_t> for SubmoduleUpdate {
    type Error = UnnamedSubmoduleUpdate;

    fn try_from(raw: ffi::git_submodule_update_t) -> Result<Self, UnnamedSubmoduleUpdate> {
        Self::from_raw(raw).ok_or(UnnamedSubmoduleUpdate(raw))
    }
}

impl From<SubmoduleUpdate> for ffi::git_submodule_update_t {
    fn from(value: SubmoduleUpdate) -> ffi::git_submodule_update_t {
        value.raw()
    }
}

/// A submodule update setting without a [`SubmoduleUpdate`] variant:
/// `GIT_SUBMODULE_UPDATE_DEFAULT` (0), or a value this version of libgit2's header
/// does not name. libgit2 refuses to write either. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedSubmoduleUpdate(ffi::git_submodule_update_t);

impl UnnamedSubmoduleUpdate {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_submodule_update_t {
        self.0
    }
}

impl fmt::Display for UnnamedSubmoduleUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 submodule update setting {} has no SubmoduleUpdate variant",
            self.0
        )
    }
}

impl std::error::Error for UnnamedSubmoduleUpdate {}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn conversions_round_trip() {
        for v in [
            SubmoduleRecurse::No,
            SubmoduleRecurse::Yes,
            SubmoduleRecurse::OnDemand,
        ] {
            let raw = ffi::git_submodule_recurse_t::from(v);
            assert_eq!(SubmoduleRecurse::try_from(raw), Ok(v));
        }
        for v in [
            SubmoduleUpdate::Checkout,
            SubmoduleUpdate::Rebase,
            SubmoduleUpdate::Merge,
            SubmoduleUpdate::None,
        ] {
            let raw = ffi::git_submodule_update_t::from(v);
            assert_eq!(SubmoduleUpdate::try_from(raw), Ok(v));
        }
    }

    #[test]
    fn defaults_are_what_an_unconfigured_submodule_reports() {
        assert_eq!(SubmoduleRecurse::default(), SubmoduleRecurse::No);
        assert_eq!(SubmoduleUpdate::default(), SubmoduleUpdate::Checkout);
        assert_eq!(
            SubmoduleUpdate::from_raw(ffi::GIT_SUBMODULE_UPDATE_DEFAULT).unwrap_or_default(),
            SubmoduleUpdate::Checkout
        );
    }

    #[test]
    fn unnamed_values_report_themselves() {
        let e = SubmoduleRecurse::try_from(3).unwrap_err();
        assert_eq!(e.raw(), 3);
        assert_eq!(
            e.to_string(),
            "libgit2 submodule recurse setting 3 is not named by git2/types.h"
        );
        assert!(e.source().is_none());

        let e = SubmoduleUpdate::try_from(ffi::GIT_SUBMODULE_UPDATE_DEFAULT).unwrap_err();
        assert_eq!(e.raw(), 0);
        assert_eq!(
            e.to_string(),
            "libgit2 submodule update setting 0 has no SubmoduleUpdate variant"
        );
        let e = SubmoduleUpdate::try_from(u32::MAX).unwrap_err();
        assert_eq!(e.raw(), u32::MAX);
        assert!(e.source().is_none());
    }
}
