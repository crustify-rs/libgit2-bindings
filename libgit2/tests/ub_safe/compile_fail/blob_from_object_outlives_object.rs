// A blob borrowed from an object cannot outlive the owner keeping that object alive.
use libgit2::libgit2::object::blob::BlobRef;
use libgit2::libgit2::object::object::ObjectArc;

fn blob<'a, 'r>(object: ObjectArc<'r>) -> Option<BlobRef<'a, 'r>> {
    BlobRef::from_object(object.as_ref())
}

fn main() {}
