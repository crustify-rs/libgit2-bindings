// An integer cannot be cast into `RefdbType`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::refs::refdb::RefdbType;

fn main() {
    let raw: u32 = 3;
    let _ = raw as RefdbType;
    let _ = 0 as RefdbType;
}
