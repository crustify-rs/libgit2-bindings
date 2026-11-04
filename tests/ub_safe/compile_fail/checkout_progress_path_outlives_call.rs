// A checkout progress callback lends its closure the path for the call only: a closure
// that would keep it, by taking a path of a lifetime it outlives, is not a callback.
use std::ffi::CStr;

use libgit2::libgit2::src::libgit2::checkout::ProgressCallback;

fn keep<'k>(kept: &mut Option<&'k CStr>) {
    let mut f = |path: Option<&'k CStr>, _: usize, _: usize| *kept = path;
    let mut callback = ProgressCallback::new(&mut f);
    callback.call(Some(c"a.txt"), 1, 1);
}

fn main() {
    let mut kept = None;
    keep(&mut kept);
    assert!(kept.is_some());
}
