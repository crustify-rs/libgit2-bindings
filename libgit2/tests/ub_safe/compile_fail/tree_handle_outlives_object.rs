// A tree viewed from an object lives no longer than the object owner it borrows, and
// a tree handle no longer than its own owner.
use libgit2::libgit2::object::object::ObjectArc;
use libgit2::libgit2::object::tree::{TreeArc, TreeRef};

fn viewed<'a, 'r>(object: ObjectArc<'r>) -> TreeRef<'a, 'r> {
    TreeRef::from_object(object.as_ref()).unwrap()
}

fn handle<'a, 'r>(tree: TreeArc<'r>) -> TreeRef<'a, 'r> {
    tree.as_ref()
}

fn main() {}
