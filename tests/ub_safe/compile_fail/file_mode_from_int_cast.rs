// An integer cannot be cast into `FileMode`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::src::libgit2::tree::FileMode;

fn main() {
    let raw: u32 = 0o100664;
    let _ = raw as FileMode;
    let _ = 0o40000 as FileMode;
}
