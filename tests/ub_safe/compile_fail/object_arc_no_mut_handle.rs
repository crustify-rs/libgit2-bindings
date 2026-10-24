// Every object is shared with its repository's cache: no owner lends an exclusive
// handle.
use {libgit2::libgit2::src::libgit2::object::ObjectArc, libgit2::libgit2::src::libgit2::object::ObjectMut};

fn exclusive<'a>(object: &'a mut ObjectArc<'static>) -> ObjectMut<'a, 'static> {
    object.as_mut()
}

fn main() {}
