// A handle cannot outlive the owner whose reference keeps the object alive.
use {libgit2::libgit2::src::libgit2::object::ObjectArc, libgit2::libgit2::src::libgit2::object::ObjectRef};

fn handle<'a, 'r>(object: ObjectArc<'r>) -> ObjectRef<'a, 'r> {
    object.as_ref()
}

fn main() {}
