// A trailer read through an array cannot outlive the array, whose block holds its
// strings.
use libgit2::libgit2::src::libgit2::trailer::MessageTrailerArray;

fn main() {
    let array = MessageTrailerArray::parse(c"Subject\n\nKey: value\n").unwrap();
    let trailers = array.as_ref().trailers();
    drop(array);
    println!("{:?}", trailers.get(0).map(|t| t.key()));
}
