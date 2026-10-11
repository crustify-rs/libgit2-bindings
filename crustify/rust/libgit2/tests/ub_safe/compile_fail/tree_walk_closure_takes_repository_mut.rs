// A walk looks subtrees up in the tree's repository, which the tree borrows shared: the
// closure cannot take the repository's exclusive handle.
use std::ffi::CStr;

use libgit2::libgit2::object::tree::{Tree, TreeEntryRef, WalkAction, WalkCallback, WalkMode};
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::RepositoryBox;

fn walk(mut repo: RepositoryBox) {
    let id = Oid::zero(OidType::Sha1);
    let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    let mut f = |_: &CStr, _: TreeEntryRef<'_>| {
        let _ = repo.as_mut();
        WalkAction::Continue
    };
    let mut callback = WalkCallback::new(&mut f);
    let _ = tree.as_ref().walk(WalkMode::PreOrder, &mut callback);
}

fn main() {
    let _ = walk;
}
