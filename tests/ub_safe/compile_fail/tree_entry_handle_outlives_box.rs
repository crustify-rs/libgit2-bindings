// A handle cannot outlive the box that frees the entry, nor can a handle's copy.
use {libgit2::libgit2::src::libgit2::tree::TreeEntryBox, libgit2::libgit2::src::libgit2::tree::TreeEntryRef};

fn handle(entry: TreeEntryBox) -> TreeEntryRef<'static> {
    let handle = entry.as_ref();
    drop(entry);
    handle
}

fn main() {}
