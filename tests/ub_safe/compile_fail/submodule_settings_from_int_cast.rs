// An integer cannot be cast into `SubmoduleRecurse` or `SubmoduleUpdate`: `from_raw`
// is the only way in, and it rejects values without a variant.
use {libgit2::libgit2::src::libgit2::submodule::SubmoduleRecurse, libgit2::libgit2::src::libgit2::submodule::SubmoduleUpdate};

fn main() {
    let raw: u32 = 0;
    let _ = raw as SubmoduleRecurse;
    let _ = 3 as SubmoduleRecurse;
    let _ = raw as SubmoduleUpdate;
    let _ = 5 as SubmoduleUpdate;
}
