// An integer cannot be cast into `PruneFlag`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::src::libgit2::worktree::PruneFlag;

fn main() {
    let raw: u32 = 3;
    let _ = raw as PruneFlag;
}
