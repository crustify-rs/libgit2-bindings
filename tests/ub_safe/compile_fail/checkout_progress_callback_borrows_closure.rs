// A checkout `ProgressCallback` borrows its closure exclusively: what the closure captures
// cannot be read while the callback can still run it.
use std::ffi::CStr;

use libgit2::libgit2::src::libgit2::checkout::ProgressCallback;

fn main() {
    let mut steps = Vec::new();
    let mut f = |_: Option<&CStr>, completed: usize, _: usize| steps.push(completed);
    let mut callback = ProgressCallback::new(&mut f);
    callback.call(None, 0, 1);
    assert_eq!(steps.len(), 1);
    callback.call(None, 1, 1);
}
