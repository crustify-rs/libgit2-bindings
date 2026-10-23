// A handle cannot outlive the owner whose reference keeps the object alive.
use libgit2::libgit2::object::object::{ObjectArc, ObjectRef};

fn handle<'a, 'r>(object: ObjectArc<'r>) -> ObjectRef<'a, 'r> {
    object.as_ref()
}

fn main() {}
