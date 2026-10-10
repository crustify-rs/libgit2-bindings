// An integer cannot be cast into `InitFlag`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::repository::repository::InitFlag;

fn main() {
    let raw: u32 = 4;
    let _ = raw as InitFlag;
}
