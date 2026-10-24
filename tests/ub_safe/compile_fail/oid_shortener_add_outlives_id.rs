// `add` keeps pointers into the id it is given: the id must outlive the shortener.
use libgit2::libgit2::src::libgit2::oid::OidShortener;

fn main() {
    let mut shortener = OidShortener::new(0).unwrap();
    {
        let id = String::from("a94a8fe5ccb19ba61c4c0873d391e987982fbbd3");
        shortener.as_mut().add(&id).unwrap();
    }
    drop(shortener);
}
