// Replacing the contents may write or free the allocation, so no view of it may still
// be live.
use libgit2::libgit2::core::buffer::Buf;

fn main() {
    let mut buf = Buf::new();
    let s = buf.as_ref().as_c_str();
    buf.as_mut().set_bytes(b"replaced");
    let _ = s.to_bytes();
}
