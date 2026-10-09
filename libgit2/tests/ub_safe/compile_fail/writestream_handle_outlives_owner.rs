// A handle lent by a `RustWriteStream` cannot outlive it: the stream's box is freed
// when the owner drops.
use libgit2::libgit2::core::types::RustWriteStream;

fn main() {
    let mut stream = RustWriteStream::new(Vec::new());
    let mut handle = stream.as_mut();
    drop(stream);
    let _ = handle.write(b"freed");
}
