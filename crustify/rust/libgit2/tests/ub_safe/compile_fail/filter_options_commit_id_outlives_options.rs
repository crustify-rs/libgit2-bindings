// The commit id the options lend is their own field: it cannot outlive them, nor be
// read while an exclusive handle writes them.
use libgit2::libgit2::object::blob::FilterOptions;
use libgit2::libgit2::oid::oid::OidRef;

fn id<'a>() -> OidRef<'a> {
    let opts = FilterOptions::new();
    opts.as_ref().attr_commit_id()
}

fn aliased() {
    let mut opts = FilterOptions::new();
    let mut id = opts.as_mut();
    let mut oid = id.attr_commit_id_mut();
    let shared = opts.as_ref().attr_commit_id();
    oid.set_raw_kind(1);
    let _ = shared.raw_kind();
}

fn main() {}
