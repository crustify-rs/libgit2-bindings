// A handle cannot outlive the box that frees the entry, nor can a handle's copy.
use libgit2::libgit2::object::tree::{TreeEntryBox, TreeEntryRef};

fn handle(entry: TreeEntryBox) -> TreeEntryRef<'static> {
    let handle = entry.as_ref();
    drop(entry);
    handle
}

fn main() {}
