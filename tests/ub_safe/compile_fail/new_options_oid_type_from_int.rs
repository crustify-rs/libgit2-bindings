// The new options' object id type is typed: a raw value libgit2 asserts on cannot be
// stored.
use libgit2::libgit2::src::libgit2::repository::NewOptions;

fn main() {
    let mut options = NewOptions::new();
    options.as_mut().set_oid_type(3u32);
}
