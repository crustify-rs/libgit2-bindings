// A handle borrows the counts it reads: it cannot outlive them.
use libgit2::libgit2::src::libgit2::checkout::{CheckoutPerfdata, CheckoutPerfdataRef};

fn main() {
    let handle: CheckoutPerfdataRef<'_>;
    {
        let counts = CheckoutPerfdata::default();
        handle = counts.as_ref();
    }
    let _ = handle.stat_calls();
}
