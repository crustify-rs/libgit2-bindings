//! Compiled pathspecs for the match-list suites, built through the safe constructor.
use libgit2::libgit2::core::strarray::StrArrayView;
use libgit2::libgit2::index::pathspec::{Pathspec, PathspecArc};
use std::ffi::CStr;

/// A shared owner, as the matchers require: each list retains its own reference.
pub fn patterns(names: &[&CStr]) -> PathspecArc {
    Pathspec::new(StrArrayView::new(names).as_ref())
        .unwrap()
        .into()
}
