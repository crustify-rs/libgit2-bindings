// A shared handle cannot coexist with the exclusive one of the same counts.
use libgit2::libgit2::src::libgit2::checkout::CheckoutPerfdata;

fn main() {
    let mut counts = CheckoutPerfdata::default();
    let shared = counts.as_ref();
    let mut exclusive = counts.as_mut();
    exclusive.set_stat_calls(1);
    let _ = shared.stat_calls();
}
