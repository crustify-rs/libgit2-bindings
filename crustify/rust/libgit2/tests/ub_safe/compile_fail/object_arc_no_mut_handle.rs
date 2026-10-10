// Every object is shared with its repository's cache: no owner lends an exclusive
// handle.
use libgit2::libgit2::object::object::{ObjectArc, ObjectMut};

fn exclusive<'a>(object: &'a mut ObjectArc<'static>) -> ObjectMut<'a, 'static> {
    object.as_mut()
}

fn main() {}
