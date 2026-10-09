// An integer cannot be cast into `LibraryOption`: `from_raw` and `from_key` are the
// only ways in, and they reject values without a variant.
use libgit2::libgit2::core::common::LibraryOption;

fn main() {
    let key: i32 = -1;
    let _ = key as LibraryOption;
    let _ = 48u32 as LibraryOption;
}
