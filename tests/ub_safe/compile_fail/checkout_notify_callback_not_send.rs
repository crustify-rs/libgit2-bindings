// A checkout `NotifyCallback` stays on the thread of its closure: libgit2 calls it
// synchronously, on the thread running the checkout, and the closure need not be `Send`.
use std::ops::ControlFlow;
use std::thread;

use libgit2::libgit2::src::libgit2::checkout::{CheckoutNotification, NotifyCallback};

fn main() {
    let mut f = |_: CheckoutNotification<'_>| ControlFlow::Continue(());
    let callback = NotifyCallback::new(&mut f);
    thread::scope(|s| {
        s.spawn(move || drop(callback));
    });
}
