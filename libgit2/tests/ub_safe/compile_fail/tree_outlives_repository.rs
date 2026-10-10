// A tree, and a clone of it, reads the repository it was looked up in: neither
// outlives it, nor does the object it is handed over as.
use libgit2::libgit2::object::tree::Tree;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::RepositoryBox;

fn escape(repo: RepositoryBox) {
    let id = Oid::zero(OidType::Sha1);
    let tree = Tree::lookup(repo.as_ref(), id.as_ref()).unwrap();
    let clone = tree.as_ref().to_tree();
    let object = Tree::into_object(tree);
    drop(repo);
    drop((clone, object));
}

fn main() {
    let _ = escape;
}
