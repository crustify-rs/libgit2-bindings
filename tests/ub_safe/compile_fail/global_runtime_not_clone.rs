// A `Runtime` is one counted reference: another one is another `init`, which can fail,
// so it does not clone.
use libgit2::libgit2::src::libgit2::libgit2::init;

fn main() {
    let r = init().unwrap();
    let _again = r.clone();
}
