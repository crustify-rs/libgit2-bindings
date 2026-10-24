// An array's trailers cannot be held while its exclusive handle takes them out.
use libgit2::libgit2::src::libgit2::trailer::MessageTrailerArray;

fn main() {
    let mut array = MessageTrailerArray::parse(c"Subject\n\nKey: value\n").unwrap();
    let key = array.as_ref().trailers().get(0).unwrap().key();
    let taken = array.as_mut().take();
    drop(taken);
    println!("{key:?}");
}
