// A handle cannot outlive the box that frees the reference.
use {libgit2::libgit2::src::libgit2::refs::ReferenceBox, libgit2::libgit2::src::libgit2::refs::ReferenceRef};

fn handle<'a, 'r>(reference: ReferenceBox<'r>) -> ReferenceRef<'a, 'r> {
    reference.as_ref()
}

fn main() {}
