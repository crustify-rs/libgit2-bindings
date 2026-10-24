// An integer cannot be cast into `Feature` or `BuildInfo`: `from_raw` is the only way
// in, and it rejects values without a variant.
use {libgit2::libgit2::src::libgit2::settings::BuildInfo, libgit2::libgit2::src::libgit2::libgit2::Feature};

fn main() {
    let raw: u32 = 3;
    let _ = raw as Feature;
    let _ = 0 as BuildInfo;
}
