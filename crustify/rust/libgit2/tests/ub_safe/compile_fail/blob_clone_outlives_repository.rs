// A blob, and every clone or copy of it, reads its repository: none outlives it.
use libgit2::libgit2::object::blob::{Blob, BlobArc};
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::Repository;

fn copies() -> (BlobArc<'static>, BlobArc<'static>) {
    let repo = Repository::open(c".").unwrap();
    let id = Oid::zero(OidType::Sha1);
    let blob = Blob::lookup(repo.as_ref(), id.as_ref()).unwrap();
    (blob.clone(), blob.as_ref().to_blob())
}

fn main() {}
