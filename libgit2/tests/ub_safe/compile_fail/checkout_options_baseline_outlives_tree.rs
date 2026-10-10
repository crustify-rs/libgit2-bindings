// The options cannot outlive the tree owner their baseline tree was borrowed from.
use libgit2::libgit2::object::tree::Tree;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::RepositoryBox;
use libgit2::libgit2::workdir::checkout::CheckoutOptions;

fn escape(repo: RepositoryBox) {
    let id = Oid::zero(OidType::Sha1);
    let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    let mut options = CheckoutOptions::new();
    options.as_mut().set_baseline(Some(tree.as_ref()));
    drop(tree);
    let _ = options.as_ref().baseline();
}

fn main() {
    let _ = escape;
}
