// The exclusive handle excludes every other: no shared handle may be held across it,
// and no second exclusive one taken while it lives.
use libgit2::libgit2::object::commit::Header;

fn main() {
    let mut header = Header::new(c"field", c"value");
    let shared = header.as_ref();
    let mut m = header.as_mut();
    m.set_value(c"other");
    let _ = shared.value();

    let mut a = header.as_mut();
    let mut b = header.as_mut();
    a.set_field(c"a");
    b.set_field(c"b");
}
