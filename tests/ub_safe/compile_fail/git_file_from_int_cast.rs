// An integer cannot be cast into `GitFile`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::src::libgit2::path::GitFile;

fn main() {
    let raw: u32 = 3;
    let _ = raw as GitFile;
    let _ = 1 as GitFile;
}
