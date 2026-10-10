// A mailmap handle cannot outlive the box that frees the mailmap.
use libgit2::libgit2::object::mailmap::Mailmap;

fn main() {
    let mailmap = Mailmap::new().unwrap();
    let handle = mailmap.as_ref();
    drop(mailmap);
    println!("{handle:?}");
}
