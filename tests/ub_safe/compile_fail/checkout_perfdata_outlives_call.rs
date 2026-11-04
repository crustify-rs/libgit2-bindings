// A checkout performance-data callback lends its closure the counters for the call only:
// a closure that would keep the handle, rather than a copy, is not a callback.
use libgit2::libgit2::src::libgit2::checkout::{
    CheckoutPerfdata, CheckoutPerfdataRef, PerfdataCallback,
};

fn keep<'k>(kept: &mut Option<CheckoutPerfdataRef<'k>>) {
    let mut f = |d: CheckoutPerfdataRef<'k>| *kept = Some(d);
    let mut callback = PerfdataCallback::new(&mut f);
    let counts = CheckoutPerfdata::default();
    callback.call(counts.as_ref());
}

fn main() {
    let mut kept = None;
    keep(&mut kept);
    assert!(kept.is_some());
}
