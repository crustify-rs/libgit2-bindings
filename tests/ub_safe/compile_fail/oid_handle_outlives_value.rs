// A handle cannot outlive the `Oid` value it borrows.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::oid::Oid;

fn main() {
    let handle = {
        let oid = Oid::zeroed();
        oid.as_ref()
    };
    let _ = handle.raw_kind();
}
