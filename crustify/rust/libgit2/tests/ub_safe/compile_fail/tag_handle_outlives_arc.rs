// A tag handle, or the object handle it views the tag as, cannot outlive the owner
// whose reference keeps the tag alive.
use libgit2::libgit2::object::object::ObjectRef;
use libgit2::libgit2::object::tag::{TagArc, TagRef};

fn handle<'a, 'r>(tag: TagArc<'r>) -> TagRef<'a, 'r> {
    tag.as_ref()
}

fn object<'a, 'r>(tag: TagArc<'r>) -> ObjectRef<'a, 'r> {
    tag.as_ref().as_object()
}

fn main() {}
