// The entry `insert_entry` lends keeps the exclusive handle borrowed: no other change,
// write or lookup through it while the entry lives.
use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::object::tree::TreeBuilderMut;
use libgit2::libgit2::oid::oid::OidRef;

fn stale(mut builder: TreeBuilderMut<'_, '_>, id: OidRef<'_>) {
    let entry = builder.insert_entry(c"a", id, FileMode::Commit).unwrap();
    builder.clear();
    let _ = builder.write();
    let _ = builder.as_ref().get(c"a");
    let _ = entry.to_tree_entry();
}

fn main() {}
