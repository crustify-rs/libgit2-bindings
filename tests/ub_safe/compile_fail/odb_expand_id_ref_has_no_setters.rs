// The shared handle only reads: setters live on `OdbExpandIdMut`.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::odb::OdbExpandId;

fn main() {
    let query = OdbExpandId::zeroed();
    let r = query.as_ref();
    r.set_length(4);
}
