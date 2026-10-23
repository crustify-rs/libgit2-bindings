// An entry a tree lends, by index, name, id or in order, lives in the tree's storage:
// none outlives the owner it was borrowed through.
use libgit2::libgit2::object::tree::TreeArc;

fn escape(tree: TreeArc<'_>) {
    let handle = tree.as_ref();
    let mut lent: Vec<_> = handle.iter().collect();
    lent.extend(handle.get(0));
    lent.extend(handle.get_by_name(c"README"));
    lent.extend(handle.get_by_id(handle.id()));
    drop(tree);
    let _ = lent.iter().map(|entry| entry.name()).count();
}

fn main() {
    let _ = escape;
}
