// There is no bare header to build or copy out: one is only valid in its container.
use {libgit2::libgit2::src::libgit2::filter::RustWriteStream, libgit2::libgit2::src::libgit2::filter::WriteStream};

fn main() {
    let _ = WriteStream::zeroed();
    let stream = RustWriteStream::new(Vec::new());
    let _copy: WriteStream = *stream.as_ref();
}
