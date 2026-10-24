// A shared handle cannot coexist with the exclusive one of the same init options.
use {libgit2::libgit2::src::libgit2::repository::InitFlags, libgit2::libgit2::src::libgit2::repository::InitOptions};

fn main() {
    let mut options = InitOptions::new();
    let shared = options.as_ref();
    let mut exclusive = options.as_mut();
    exclusive.set_flags(InitFlags::empty());
    let _ = shared.flags();
}
