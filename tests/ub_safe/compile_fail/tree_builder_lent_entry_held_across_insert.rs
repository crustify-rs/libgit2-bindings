// An entry `get` lends cannot be held while the builder replaces or removes entries,
// which may rewrite or free it.
use libgit2::libgit2::src::libgit2::tree::FileMode;
use libgit2::libgit2::src::libgit2::tree::TreeBuilderBox;
use libgit2::libgit2::src::libgit2::oid::OidRef;

fn stale(mut builder: TreeBuilderBox<'_>, id: OidRef<'_>) {
    let entry = builder.as_ref().get(c"a").unwrap();
    builder.as_mut().insert(c"a", id, FileMode::Commit).unwrap();
    builder.as_mut().remove(c"a").unwrap();
    let _ = entry.to_tree_entry();
}

fn main() {}
