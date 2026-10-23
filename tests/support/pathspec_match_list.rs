//! Compiled pathspecs for the match-list suites, built through the safe constructor.
use std::ffi::CStr;

use libgit2::libgit2::core::strarray::StrArrayView;
use libgit2::libgit2::index::pathspec::{Pathspec, PathspecArc};

/// A shared owner, as the matchers require: each list retains its own reference.
pub(super) fn patterns(names: &[&CStr]) -> PathspecArc {
    Pathspec::new(StrArrayView::new(names).as_ref())
        .unwrap()
        .into()
}
