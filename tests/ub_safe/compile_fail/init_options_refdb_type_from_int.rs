// The init options' reference storage is typed: a raw value libgit2 does not name
// cannot be stored.
use libgit2::libgit2::repository::repository::InitOptions;

fn main() {
    let mut options = InitOptions::new();
    options.as_mut().set_refdb_type(3u32);
}
