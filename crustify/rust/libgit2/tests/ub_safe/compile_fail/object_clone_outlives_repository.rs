// A clone or a handle's copy of an object is the same object, tied to the same
// repository.
use libgit2::libgit2::object::object::{Object, ObjectArc};
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::RepositoryBox;

fn escape(repo: RepositoryBox) -> (ObjectArc<'static>, ObjectArc<'static>) {
    let id = Oid::zero(OidType::Sha1);
    let object = Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap();
    (object.clone(), object.as_ref().to_object())
}

fn main() {
    let _ = escape;
}
