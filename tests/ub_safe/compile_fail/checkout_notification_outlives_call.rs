// A checkout notification callback lends its closure the notification for the call only:
// a closure that would keep it, by taking a notification of a lifetime it outlives, is not
// a callback.
use std::ops::ControlFlow;

use libgit2::libgit2::src::libgit2::checkout::{CheckoutNotification, NotifyCallback};

fn keep<'k>(kept: &mut Option<CheckoutNotification<'k>>) {
    let mut f = |n: CheckoutNotification<'k>| {
        *kept = Some(n);
        ControlFlow::Continue(())
    };
    let mut callback = NotifyCallback::new(&mut f);
    let n = CheckoutNotification { why: 1, path: None, baseline: None, target: None, workdir: None };
    let _ = callback.call(n);
}

fn main() {
    let mut kept = None;
    keep(&mut kept);
    assert!(kept.is_some());
}
