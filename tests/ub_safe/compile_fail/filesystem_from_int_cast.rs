// An integer cannot be cast into `Filesystem`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::src::libgit2::path::Filesystem;

fn main() {
    let raw: u32 = 1;
    let _ = raw as Filesystem;
    let _ = 3 as Filesystem;
}
