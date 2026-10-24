// The init options' object id type is typed: a raw value libgit2 does not name (which
// it would record as `unknown`, leaving an unopenable repository) cannot be stored.
use libgit2::libgit2::src::libgit2::repository::InitOptions;

fn main() {
    let mut options = InitOptions::new();
    options.as_mut().set_oid_type(3u32);
}
