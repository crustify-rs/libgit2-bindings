// A reference cannot outlive the repository borrow it was read through: its owner
// carries the borrow's lifetime, which its drop still needs. `read` has the shape of
// a lookup wrapper.
use libgit2::libgit2::refs::refs::ReferenceBox;

struct Repository;

fn read<'r>(_repository: &'r Repository) -> ReferenceBox<'r> {
    unimplemented!()
}

fn main() {
    let reference = {
        let repository = Repository;
        read(&repository)
    };
    drop(reference);
}
