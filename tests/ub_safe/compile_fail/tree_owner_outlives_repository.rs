// A tree's repository is the handle it was looked up through: it outlives the tree,
// but not the repository.
use libgit2::libgit2::src::libgit2::tree::Tree;
use {libgit2::libgit2::src::libgit2::oid::Oid, libgit2::libgit2::src::libgit2::oid::OidType};
use libgit2::libgit2::src::libgit2::repository::RepositoryBox;

fn escape(repo: RepositoryBox) {
    let id = Oid::zero(OidType::Sha1);
    let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    let owner = tree.as_ref().owner();
    drop(tree);
    drop(repo);
    let _ = owner.path();
}

fn main() {
    let _ = escape;
}
