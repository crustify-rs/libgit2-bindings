// A `WriteStreamBox` carries no lifetime, so a stream erased into one cannot borrow.
use libgit2::libgit2::core::types::{RustWriteStream, WriteStreamBox};

fn erased() -> WriteStreamBox {
    let mut out = Vec::new();
    RustWriteStream::new(&mut out).into_box()
}

fn main() {
    let _ = erased();
}
