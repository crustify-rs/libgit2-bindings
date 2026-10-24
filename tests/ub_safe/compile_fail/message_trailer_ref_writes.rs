// The shared trailer handle only reads.
use libgit2::libgit2::src::libgit2::trailer::MessageTrailer;

fn main() {
    let trailer = MessageTrailer::new(c"Fixes", c"#1");
    let shared = trailer.as_ref();
    shared.set_value(c"#2");
}
