// A tag, and a clone of it, reads the repository it was looked up in: neither outlives
// it, nor does the object it is handed over as.
use libgit2::libgit2::src::libgit2::tag::Tag;
use {libgit2::libgit2::src::libgit2::oid::Oid, libgit2::libgit2::src::libgit2::oid::OidType};
use libgit2::libgit2::src::libgit2::repository::RepositoryBox;

fn escape(repo: RepositoryBox) {
    let id = Oid::zero(OidType::Sha1);
    let tag = Tag::lookup_prefix(repo.as_ref(), id.as_ref(), 7).unwrap();
    let clone = tag.as_ref().to_tag();
    let object = Tag::into_object(tag);
    drop(repo);
    drop((clone, object));
}

fn main() {
    let _ = escape;
}
