// Every tag and tree is shared with its repository's cache: no owner lends an
// exclusive handle.
use {libgit2::libgit2::src::libgit2::tag::TagArc, libgit2::libgit2::src::libgit2::tag::TagMut};
use {libgit2::libgit2::src::libgit2::tree::TreeArc, libgit2::libgit2::src::libgit2::tree::TreeMut};

fn tag<'a>(tag: &'a mut TagArc<'static>) -> TagMut<'a, 'static> {
    tag.as_mut()
}

fn tree<'a>(tree: &'a mut TreeArc<'static>) -> TreeMut<'a, 'static> {
    tree.as_mut()
}

fn main() {}
