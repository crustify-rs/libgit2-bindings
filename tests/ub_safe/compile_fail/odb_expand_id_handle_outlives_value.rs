// A handle cannot outlive the `OdbExpandId` query it borrows, nor the id handle the query lends.
use ffibox::CZeroable;
use libgit2::libgit2::src::libgit2::odb::OdbExpandId;

fn main() {
    let id = {
        let query = OdbExpandId::zeroed();
        query.as_ref().id()
    };
    let _ = id.raw_kind();
}
