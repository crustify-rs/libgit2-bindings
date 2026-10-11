//! A scan's callback cannot reach the index it scans: the scan holds the exclusive
//! handle for the whole call.
use std::ffi::CStr;

use libgit2::libgit2::core::strarray::StrArrayView;
use libgit2::libgit2::index::index::{AddFlags, Index, MatchedPathCallback, MatchedPathResult};

fn main() {
    let mut index = Index::new().unwrap();
    let mut f = |_: &CStr, _: Option<&CStr>| {
        let _ = index.as_ref().entry_count();
        MatchedPathResult::APPLY
    };
    let mut callback = MatchedPathCallback::new(&mut f);
    let pathspec = StrArrayView::default();
    let _ = index
        .as_mut()
        .add_all(pathspec.as_ref(), AddFlags::DEFAULT, Some(&mut callback));
}
