// An integer does not cast into an `UpdateAction`: a value libgit2's header does not
// name would be an invalid enum; `from_raw` / `try_from` check it.
use libgit2::libgit2::object::tree::UpdateAction;

fn main() {
    let _ = 2u32 as UpdateAction;
}
