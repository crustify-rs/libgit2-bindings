// A clone or a handle's copy of an object is the same object, tied to the same
// repository.
use {libgit2::libgit2::src::libgit2::object::Object, libgit2::libgit2::src::libgit2::object::ObjectArc};
use {libgit2::libgit2::src::libgit2::oid::Oid, libgit2::libgit2::src::libgit2::oid::OidType};
use libgit2::libgit2::src::libgit2::repository::RepositoryBox;

fn escape(repo: RepositoryBox) -> (ObjectArc<'static>, ObjectArc<'static>) {
    let id = Oid::zero(OidType::Sha1);
    let object = Object::lookup(repo.as_ref(), id.as_ref(), None).unwrap();
    (object.clone(), object.as_ref().to_object())
}

fn main() {
    let _ = escape;
}
