//! A shared index hands out only shared handles: another holder may change the index
//! behind it.
use {libgit2::libgit2::src::libgit2::index::Index, libgit2::libgit2::src::libgit2::index::IndexArc, libgit2::libgit2::src::libgit2::index::IndexMut};

fn main() {
    let index: IndexArc = Index::new().unwrap().into();
    let _: IndexMut<'_> = index.as_ref();
}
