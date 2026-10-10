// Every tag and tree is shared with its repository's cache: no owner lends an
// exclusive handle.
use libgit2::libgit2::object::tag::{TagArc, TagMut};
use libgit2::libgit2::object::tree::{TreeArc, TreeMut};

fn tag<'a>(tag: &'a mut TagArc<'static>) -> TagMut<'a, 'static> {
    tag.as_mut()
}

fn tree<'a>(tree: &'a mut TreeArc<'static>) -> TreeMut<'a, 'static> {
    tree.as_mut()
}

fn main() {}
