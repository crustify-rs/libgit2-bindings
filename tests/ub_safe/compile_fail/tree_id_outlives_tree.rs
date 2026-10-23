// A tree's id lives inside the tree: it does not outlive the owner it was read through.
use libgit2::libgit2::object::tree::TreeArc;

fn escape(tree: TreeArc<'_>) {
    let handle = tree.as_ref();
    let id = handle.id();
    drop(tree);
    let _ = id.id();
}

fn main() {
    let _ = escape;
}
