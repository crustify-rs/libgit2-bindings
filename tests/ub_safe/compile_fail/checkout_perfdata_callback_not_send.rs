// A checkout `PerfdataCallback` stays on the thread of its closure: libgit2 calls it
// synchronously, on the thread running the checkout, and the closure need not be `Send`.
use std::thread;

use libgit2::libgit2::src::libgit2::checkout::{CheckoutPerfdataRef, PerfdataCallback};

fn main() {
    let mut f = |_: CheckoutPerfdataRef<'_>| {};
    let callback = PerfdataCallback::new(&mut f);
    thread::scope(|s| {
        s.spawn(move || drop(callback));
    });
}
