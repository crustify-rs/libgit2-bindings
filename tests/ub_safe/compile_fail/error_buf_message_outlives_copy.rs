// `last` returns an owned copy; its message is borrowed from that copy, and cannot
// outlive it.
use libgit2::libgit2::src::util::errors::last;

fn main() {
    let message = {
        let error = last();
        error.message()
    };
    println!("{message:?}");
}
