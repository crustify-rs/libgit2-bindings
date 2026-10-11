//! A shared index has no exclusive handle, so nothing frees its entries behind the
//! other holders.
use libgit2::libgit2::index::index::{Index, IndexArc};

fn main() {
    let index: IndexArc = Index::new().unwrap().into();
    let _ = index.as_ref().clear();
}
