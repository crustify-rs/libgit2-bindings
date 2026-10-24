// The borrowed run of ids cannot outlive the array that owns them.
use libgit2::libgit2::src::libgit2::oidarray::OidArray;

fn main() {
    let ids = {
        let array = OidArray::new();
        array.as_ref().ids()
    };
    let _ = ids.len();
}
