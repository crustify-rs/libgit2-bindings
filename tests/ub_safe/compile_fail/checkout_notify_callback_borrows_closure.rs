// A checkout `NotifyCallback` borrows its closure exclusively: what the closure captures
// cannot be read while the callback can still run it.
use std::ops::ControlFlow;

use libgit2::libgit2::src::libgit2::checkout::{CheckoutNotification, NotifyCallback};

fn main() {
    let mut whys = Vec::new();
    let mut f = |n: CheckoutNotification<'_>| {
        whys.push(n.why);
        ControlFlow::Continue(())
    };
    let mut callback = NotifyCallback::new(&mut f);
    let n = CheckoutNotification { why: 1, path: None, baseline: None, target: None, workdir: None };
    let _ = callback.call(n);
    assert_eq!(whys.len(), 1);
    let _ = callback.call(n);
}
