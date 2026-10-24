// An integer cannot be cast into `RepositoryState`: `from_raw` is the only way in, and
// it rejects values without a variant.
use libgit2::libgit2::src::libgit2::repository::RepositoryState;

fn main() {
    let raw: u32 = 12;
    let _ = raw as RepositoryState;
    let _ = -1 as RepositoryState;
}
