// A handle cannot outlive the box that frees the shortener.
use libgit2::libgit2::src::libgit2::oid::OidShortener;

fn main() {
    let handle = {
        let mut shortener = OidShortener::new(0).unwrap();
        shortener.as_mut()
    };
    let _ = handle.as_ref();
}
