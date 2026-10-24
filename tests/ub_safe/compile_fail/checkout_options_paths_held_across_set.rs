// The paths the getter returns live in the options: no setter while they are held.
use libgit2::libgit2::src::libgit2::checkout::CheckoutOptions;

fn main() {
    let mut options = CheckoutOptions::new();
    let paths = options.as_ref().paths();
    options.as_mut().clear_paths();
    let _ = paths.len();
}
