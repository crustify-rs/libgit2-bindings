// The exclusive run of id bytes freezes its handle: no write through both.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::oid::Oid;

fn main() {
    let mut oid = Oid::zeroed();
    let mut m = oid.as_mut();
    let mut id = m.id_mut();
    m.set_id([1; 32]);
    id.set_elem(0, 2);
}
