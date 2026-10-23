// Taking the contents out moves the allocation away, so no view of it may still be
// live.
use libgit2::libgit2::core::buffer::Buf;

fn main() {
    let mut buf = Buf::new();
    let r = buf.as_ref();
    let taken = buf.as_mut().take_bytes();
    let _ = (r.as_bytes(), taken);
}
