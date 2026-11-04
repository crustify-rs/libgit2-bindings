// A stored callback is borrowed exclusively: it cannot be called beside the options that
// may call it.
use std::ffi::CStr;
use std::ops::ControlFlow;

use libgit2::libgit2::src::libgit2::checkout::{
    CheckoutNotification, CheckoutOptions, NotifyCallback,
};

fn main() {
    let mut record = |_: CheckoutNotification<'_>| ControlFlow::Continue(());
    let mut callback = NotifyCallback::new(&mut record);
    let mut options = CheckoutOptions::new();
    options.as_mut().set_notify_callback(Some(&mut callback));
    let note = CheckoutNotification {
        why: 0,
        path: Some(<&CStr>::default()),
        baseline: None,
        target: None,
        workdir: None,
    };
    let _ = callback.call(note);
    let _ = options.as_ref().notify_callback();
}
