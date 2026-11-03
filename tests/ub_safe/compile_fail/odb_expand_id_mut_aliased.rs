// The id lent in place by `id_mut` borrows the query exclusively: no other handle to the query
// coexists with it.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::odb::OdbExpandId;

fn main() {
    let mut query = OdbExpandId::zeroed();
    let mut m = query.as_mut();
    let mut id = m.id_mut();
    m.set_length(4);
    let r = query.as_ref();
    id.set_raw_kind(1);
    let _ = r.length();
}
