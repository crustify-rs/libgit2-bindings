// An integer cannot be cast into `WalkMode`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::object::tree::WalkMode;

fn main() {
    let raw: u32 = 2;
    let _ = raw as WalkMode;
    let _ = 0 as WalkMode;
}
