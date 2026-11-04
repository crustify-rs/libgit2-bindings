// The options borrow a callback they store for their whole lifetime: it cannot be dropped
// while they may still call it.
use std::ffi::CStr;

use libgit2::libgit2::src::libgit2::checkout::{CheckoutOptions, ProgressCallback};

fn main() {
    let mut record = |_: Option<&CStr>, _: usize, _: usize| {};
    let mut callback = ProgressCallback::new(&mut record);
    let mut options = CheckoutOptions::new();
    options.as_mut().set_progress_callback(Some(&mut callback));
    drop(callback);
    let _ = options.as_ref().progress_callback();
}
