// The writer cannot be borrowed while a handle to its stream is live: the handle's
// `write` runs the writer through the stream's member.
use libgit2::libgit2::src::libgit2::filter::RustWriteStream;

fn main() {
    let mut stream = RustWriteStream::new(Vec::new());
    let mut handle = stream.as_mut();
    let writer = stream.writer();
    handle.write(b"aliased").unwrap();
    let _ = writer;
}
