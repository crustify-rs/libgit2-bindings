// The exclusive handle excludes every other: no shared handle, or id handle, may be
// held across it, and no second exclusive one taken while it lives.
use libgit2::libgit2::src::libgit2::tree::FileMode;
use libgit2::libgit2::src::libgit2::tree::TreeUpdate;

fn main() {
    let mut update = TreeUpdate::remove(c"README");
    let shared = update.as_ref();
    let mut m = update.as_mut();
    m.set_filemode(FileMode::Blob);
    let _ = shared.path();

    let id = update.as_ref().id();
    let mut m = update.as_mut();
    m.id_mut().set_raw_kind(1);
    let _ = id.raw_kind();

    let mut a = update.as_mut();
    let mut b = update.as_mut();
    a.set_path(c"a");
    b.set_path(c"b");
}
