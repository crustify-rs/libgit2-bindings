//! Compiled pathspecs for the match-list suites, built through the safe constructor.
use std::ffi::CStr;

use libgit2::libgit2::src::libgit2::pathspec::{Pathspec, PathspecArc};
use libgit2::libgit2::src::libgit2::strarray::StrArrayView;

/// A shared owner, as the matchers require: each list retains its own reference.
pub(super) fn patterns(names: &[&CStr]) -> PathspecArc {
    Pathspec::new(StrArrayView::new(names).as_ref())
        .unwrap()
        .into()
}
