// A blob, and every clone or copy of it, reads its repository: none outlives it.
use {libgit2::libgit2::src::libgit2::blob::Blob, libgit2::libgit2::src::libgit2::blob::BlobArc};
use {libgit2::libgit2::src::libgit2::oid::Oid, libgit2::libgit2::src::libgit2::oid::OidType};
use libgit2::libgit2::src::libgit2::repository::Repository;

fn copies() -> (BlobArc<'static>, BlobArc<'static>) {
    let repo = Repository::open(c".").unwrap();
    let id = Oid::zero(OidType::Sha1);
    let blob = Blob::lookup(repo.as_ref(), id.as_ref()).unwrap();
    (blob.clone(), blob.as_ref().to_blob())
}

fn main() {}
