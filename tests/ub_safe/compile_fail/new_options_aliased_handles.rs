// A shared handle cannot coexist with the exclusive one of the same new options.
use libgit2::libgit2::src::libgit2::repository::NewOptions;

fn main() {
    let mut options = NewOptions::new();
    let shared = options.as_ref();
    let mut exclusive = options.as_mut();
    exclusive.set_oid_type(None);
    let _ = shared.oid_type();
}
