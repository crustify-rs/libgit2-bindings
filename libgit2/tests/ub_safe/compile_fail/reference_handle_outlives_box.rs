// A handle cannot outlive the box that frees the reference.
use libgit2::libgit2::refs::refs::{ReferenceBox, ReferenceRef};

fn handle<'a, 'r>(reference: ReferenceBox<'r>) -> ReferenceRef<'a, 'r> {
    reference.as_ref()
}

fn main() {}
