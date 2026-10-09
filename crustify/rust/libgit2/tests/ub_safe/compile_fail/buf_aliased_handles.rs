// Two exclusive handles to one buffer cannot coexist.
use libgit2::libgit2::core::buffer::Buf;

fn main() {
    let mut buf = Buf::new();
    let mut a = buf.as_mut();
    let mut b = buf.as_mut();
    a.set_bytes(b"a");
    b.set_bytes(b"b");
}
