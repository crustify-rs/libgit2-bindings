// The name and id an entry lends live inside its block, which the box frees.
use libgit2::libgit2::src::libgit2::tree::TreeEntryBox;
use libgit2::libgit2::src::libgit2::oid::OidRef;
use std::ffi::CStr;

fn name(entry: TreeEntryBox) -> &'static CStr {
    let name = entry.as_ref().name();
    drop(entry);
    name
}

fn id(entry: TreeEntryBox) -> OidRef<'static> {
    let id = entry.as_ref().id();
    drop(entry);
    id
}

fn main() {
    let _ = (name, id);
}
