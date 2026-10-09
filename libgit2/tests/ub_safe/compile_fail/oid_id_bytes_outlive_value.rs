// The borrowed run of id bytes cannot outlive the `Oid` it projects.
use ffibox::CZeroable;
use libgit2::libgit2::oid::oid::Oid;

fn main() {
    let bytes = {
        let oid = Oid::zeroed();
        oid.as_ref().id_bytes()
    };
    let _ = bytes.elem(0);
}
