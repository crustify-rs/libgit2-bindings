// Two exclusive handles to one reference cannot coexist, nor a shared one beside an
// exclusive one.
use libgit2::libgit2::refs::refs::UnattachedReference;

fn main() {
    let mut reference = UnattachedReference::symbolic(c"HEAD", c"refs/heads/main").unwrap();
    let first = reference.as_mut();
    let second = reference.as_mut();
    drop((first, second));
    let exclusive = reference.as_mut();
    let shared = reference.as_ref();
    drop((exclusive, shared));
}
