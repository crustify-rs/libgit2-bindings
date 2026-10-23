// An entry `get` lends is the builder's own: it cannot outlive the box that frees it.
use libgit2::libgit2::object::tree::{TreeBuilderBox, TreeEntryRef};

fn lent(builder: TreeBuilderBox<'_>) -> TreeEntryRef<'static> {
    let entry = builder.as_ref().get(c"a").unwrap();
    drop(builder);
    entry
}

fn main() {}
