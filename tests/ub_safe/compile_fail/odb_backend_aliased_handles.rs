// A shared handle cannot live across a call through the exclusive one: the member
// may change the backend the shared handle reads.
use libgit2::libgit2::src::libgit2::odb::OdbBackend;

fn main() {
    let mut backend = OdbBackend::new();
    let shared = backend.as_ref();
    backend.as_mut().refresh().unwrap();
    let _ = shared.version();
}
