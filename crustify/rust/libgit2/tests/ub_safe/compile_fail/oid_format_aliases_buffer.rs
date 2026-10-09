// A formatter's result borrows its buffer: the buffer cannot be written while it
// lives.
use ffibox::CZeroable;
use libgit2::libgit2::oid::oid::Oid;

fn main() {
    let oid = Oid::zeroed();
    let mut buf = [0u8; 64];
    let hex = oid.as_ref().format(&mut buf).unwrap();
    buf[0] = b'x';
    println!("{hex} {}", buf[0]);
}
