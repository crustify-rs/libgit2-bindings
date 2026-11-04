// One callback cannot be stored in two options at once: each borrows it exclusively.
use libgit2::libgit2::src::libgit2::checkout::{
    CheckoutOptions, CheckoutPerfdataRef, PerfdataCallback,
};

fn main() {
    let mut record = |_: CheckoutPerfdataRef<'_>| {};
    let mut callback = PerfdataCallback::new(&mut record);
    let mut first = CheckoutOptions::new();
    let mut second = CheckoutOptions::new();
    first.as_mut().set_perfdata_callback(Some(&mut callback));
    second.as_mut().set_perfdata_callback(Some(&mut callback));
    let _ = (first.as_ref().perfdata_callback(), second.as_ref().perfdata_callback());
}
