// A handle borrows the header value it was taken from.
use libgit2::libgit2::object::commit::{Header, HeaderRef};

fn handle<'a>() -> HeaderRef<'a, 'static> {
    let header = Header::new(c"field", c"value");
    header.as_ref()
}

fn main() {
    let _ = handle().field();
}
