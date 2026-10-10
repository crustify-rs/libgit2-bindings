// An object reads the repository it was looked up in: it cannot outlive it.
use libgit2::libgit2::object::object::Object;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::RepositoryBox;

fn escape(repo: RepositoryBox) {
    let id = Oid::zero(OidType::Sha1);
    let object = Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap();
    drop(repo);
    drop(object);
}

fn main() {
    let _ = escape;
}
