//! Positions come from the exclusive handle: libgit2 sorts the entries to find one.
use libgit2::libgit2::src::libgit2::index::Index;

fn main() {
    let index = Index::new().unwrap();
    let _ = index.as_ref().find(c"a");
}
