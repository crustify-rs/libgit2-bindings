// An integer cannot be cast into `FilterFlag`: `from_raw` is the only way in, and it
// rejects values without a variant.
use libgit2::libgit2::src::libgit2::blob::FilterFlag;

fn main() {
    let raw: u32 = 1;
    let _ = raw as FilterFlag;
    let _ = 16 as FilterFlag;
}
