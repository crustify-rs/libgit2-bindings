// The object a blob is, borrowed, cannot outlive the blob's owner.
use libgit2::libgit2::src::libgit2::blob::BlobArc;
use libgit2::libgit2::src::libgit2::object::ObjectRef;

fn object<'a, 'r>(blob: BlobArc<'r>) -> ObjectRef<'a, 'r> {
    blob.as_ref().as_object()
}

fn main() {}
