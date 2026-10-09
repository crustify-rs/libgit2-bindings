// An integer cannot be cast into `BranchType`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::core::types::BranchType;

fn main() {
    let raw: u32 = 0;
    let _ = raw as BranchType;
    let _ = 4 as BranchType;
}
