// A walk looks subtrees up in the tree's repository, which the tree borrows shared: the
// closure cannot take the repository's exclusive handle.
use std::ffi::CStr;

use {libgit2::libgit2::src::libgit2::tree::Tree, libgit2::libgit2::src::libgit2::tree::TreeEntryRef, libgit2::libgit2::src::libgit2::tree::WalkAction, libgit2::libgit2::src::libgit2::tree::WalkCallback, libgit2::libgit2::src::libgit2::tree::WalkMode};
use {libgit2::libgit2::src::libgit2::oid::Oid, libgit2::libgit2::src::libgit2::oid::OidType};
use libgit2::libgit2::src::libgit2::repository::RepositoryBox;

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
