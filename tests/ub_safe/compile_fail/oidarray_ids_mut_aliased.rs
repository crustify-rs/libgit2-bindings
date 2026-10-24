// The exclusive run of ids freezes its handle: no shared view alongside it.
use libgit2::libgit2::src::libgit2::oidarray::OidArray;

fn main() {
    let mut array = OidArray::new();
    let mut m = array.as_mut();
    let mut run = m.ids_mut();
    let shared = m.as_ref();
    let _ = run.get_mut(0);
    let _ = shared.len();
}
