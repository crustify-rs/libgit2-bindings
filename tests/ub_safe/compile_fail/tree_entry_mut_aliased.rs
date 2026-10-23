// Two exclusive handles to one entry cannot coexist, nor a shared one beside an
// exclusive one.
use libgit2::libgit2::object::tree::TreeEntryBox;

fn alias(mut entry: TreeEntryBox) {
    let a = entry.as_mut();
    let b = entry.as_mut();
    let c = entry.as_ref();
    drop((a, b, c));
}

fn main() {}
