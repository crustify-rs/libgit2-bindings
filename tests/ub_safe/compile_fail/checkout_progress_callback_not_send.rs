// A checkout `ProgressCallback` stays on the thread of its closure: libgit2 calls it
// synchronously, on the thread running the checkout, and the closure need not be `Send`.
use std::ffi::CStr;
use std::thread;

use libgit2::libgit2::src::libgit2::checkout::ProgressCallback;

fn main() {
    let mut f = |_: Option<&CStr>, _: usize, _: usize| {};
    let callback = ProgressCallback::new(&mut f);
    thread::scope(|s| {
        s.spawn(move || drop(callback));
    });
}
