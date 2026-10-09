// The borrowed contents cannot outlive the buffer that owns them.
use libgit2::libgit2::core::buffer::Buf;

fn main() {
    let bytes = {
        let buf = Buf::new();
        buf.as_ref().as_bytes()
    };
    let _ = bytes.len();
}
