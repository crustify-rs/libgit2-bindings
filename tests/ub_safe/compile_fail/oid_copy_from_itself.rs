// `git_oid_cpy` is a `memcpy`: its source cannot be its destination.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::oid::Oid;

fn main() {
    let mut oid = Oid::zeroed();
    let src = oid.as_ref();
    oid.as_mut().copy_from(src);
}
