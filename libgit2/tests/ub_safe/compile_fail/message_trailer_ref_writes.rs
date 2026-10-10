// The shared trailer handle only reads.
use libgit2::libgit2::object::message::MessageTrailer;

fn main() {
    let trailer = MessageTrailer::new(c"Fixes", c"#1");
    let shared = trailer.as_ref();
    shared.set_value(c"#2");
}
