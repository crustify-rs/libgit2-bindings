// Closing consumes the owner: a closed stream accepts nothing but `free`, so it can
// no longer be written.
use {libgit2::libgit2::src::libgit2::filter::RustWriteStream, libgit2::libgit2::src::libgit2::filter::WriteStream};

fn main() {
    let mut stream = RustWriteStream::new(Vec::new()).into_box();
    WriteStream::close(stream).unwrap();
    let _ = stream.as_mut().write(b"after close");
}
