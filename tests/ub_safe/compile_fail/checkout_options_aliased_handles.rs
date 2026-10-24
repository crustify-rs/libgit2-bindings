// A shared handle cannot coexist with the exclusive one of the same options.
use libgit2::libgit2::src::libgit2::checkout::CheckoutOptions;

fn main() {
    let mut options = CheckoutOptions::new();
    let shared = options.as_ref();
    let mut exclusive = options.as_mut();
    exclusive.set_strategy(2);
    let _ = shared.strategy();
}
