// An integer cannot be cast into `OidType`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::src::libgit2::oid::OidType;

fn main() {
    let raw: u32 = 0;
    let _ = raw as OidType;
    let _ = 3 as OidType;
}
