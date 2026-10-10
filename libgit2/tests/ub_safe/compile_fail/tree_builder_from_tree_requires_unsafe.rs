// `TreeBuilder::from_tree` takes a raw `git_tree` pointer, whose liveness the caller
// vouches for: it is an `unsafe fn`.
#![deny(unsafe_code)]

use core::ptr::NonNull;

use libgit2::ffi;
use libgit2::libgit2::object::tree::TreeBuilder;
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let repo = Repository::new().unwrap();
    let dangling = NonNull::<ffi::git_tree>::dangling();
    let _ = TreeBuilder::from_tree(repo.as_ref(), dangling);
}
