// A handle, and the id handle projected from it, borrow the update they were taken
// from.
use {libgit2::libgit2::src::libgit2::tree::TreeUpdate, libgit2::libgit2::src::libgit2::tree::TreeUpdateRef};
use libgit2::libgit2::src::libgit2::oid::OidRef;

fn handle<'a>() -> TreeUpdateRef<'a, 'static> {
    let update = TreeUpdate::remove(c"README");
    update.as_ref()
}

fn id<'a>() -> OidRef<'a> {
    let update = TreeUpdate::remove(c"README");
    update.as_ref().id()
}

fn id_mut_outlives() {
    let mut update = TreeUpdate::remove(c"README");
    let mut m = update.as_mut();
    let id = m.id_mut();
    drop(m);
    drop(id);
}

fn main() {
    let _ = handle().path();
    let _ = id().raw_kind();
    id_mut_outlives();
}
