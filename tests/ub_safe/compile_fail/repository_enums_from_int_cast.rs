// An integer cannot be cast into `Item` or `OpenFlag`: `from_raw` is the only way in,
// and it rejects values without a variant, `GIT_REPOSITORY_ITEM__LAST` included.
use {libgit2::libgit2::src::libgit2::repository::Item, libgit2::libgit2::src::libgit2::repository::OpenFlag};

fn main() {
    let raw: u32 = 15;
    let _ = raw as Item;
    let _ = 1 as OpenFlag;
}
