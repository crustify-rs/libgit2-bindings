// Replacing the ids frees the old buffer, so no run over it may still be live.
use libgit2::libgit2::src::libgit2::oidarray::OidArray;
use libgit2::libgit2::src::libgit2::odb::OdbDataVec;

fn main() {
    let mut array = OidArray::new();
    let ids = array.as_ref().ids();
    array.as_mut().set_ids(OdbDataVec::empty());
    let _ = ids.get(0);
}
