// A shared mailmap handle cannot be used while an exclusive one lives.
use libgit2::libgit2::object::mailmap::Mailmap;

fn main() {
    let mut mailmap = Mailmap::from_buffer(b"Real <r@x> <o@x>").unwrap();
    let writer = mailmap.as_mut();
    let reader = mailmap.as_ref();
    println!("{reader:?}");
    drop(writer);
}
