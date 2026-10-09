// The shortener reads the ids it keeps on later additions: an id cannot be written
// while the shortener lives.
use libgit2::libgit2::oid::oid::OidShortener;

fn main() {
    let mut id = String::from("a94a8fe5ccb19ba61c4c0873d391e987982fbbd3");
    let mut shortener = OidShortener::new(0).unwrap();
    shortener.as_mut().add(&id).unwrap();
    id.replace_range(..1, "b");
    shortener.as_mut().add("a94a8fe5ccb19ba61c4c0873d391e987982fbbd4").unwrap();
}
