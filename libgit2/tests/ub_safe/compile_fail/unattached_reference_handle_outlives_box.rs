// A handle cannot outlive the box that frees the reference.
use libgit2::libgit2::refs::refs::UnattachedReference;

fn main() {
    let handle = {
        let mut reference = UnattachedReference::symbolic(c"HEAD", c"refs/heads/main").unwrap();
        reference.as_mut()
    };
    let _ = handle.as_ref();
}
