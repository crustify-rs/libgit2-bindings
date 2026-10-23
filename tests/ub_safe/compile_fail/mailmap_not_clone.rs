// libgit2 has no copy routine for a mailmap, so its box does not clone.
use libgit2::libgit2::object::mailmap::Mailmap;

fn main() {
    let mailmap = Mailmap::new().unwrap();
    let _copy = mailmap.clone();
}
