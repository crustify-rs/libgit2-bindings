// A handle cannot outlive the backend it borrows.
use libgit2::libgit2::src::libgit2::odb::{OdbBackend, OdbBackendRef};

fn main() {
    let handle: OdbBackendRef<'_> = {
        let backend = OdbBackend::new();
        backend.as_ref()
    };
    let _ = handle.version();
}
