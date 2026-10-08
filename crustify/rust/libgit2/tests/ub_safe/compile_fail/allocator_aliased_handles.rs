// A shared handle cannot coexist with the exclusive one of the same `Allocator`.
use libgit2::libgit2::util::sys::alloc::{Allocator, FreeFn};

fn replace_free(allocator: &mut Allocator, free: FreeFn) {
    let shared = allocator.as_ref();
    let mut exclusive = allocator.as_mut();
    exclusive.set_free(free);
    let _ = shared.free();
}

fn main() {
    let _ = replace_free;
}
