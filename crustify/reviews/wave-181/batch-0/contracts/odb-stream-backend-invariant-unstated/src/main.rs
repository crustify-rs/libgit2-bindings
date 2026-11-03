//! `OdbStreamRef::from_ptr`'s documented obligation (ffibox's `CCell::ref_from_ptr`: a live,
//! initialised `git_odb_stream` that outlives the handle and that nothing writes meanwhile)
//! is honoured below. Before this review, no documentation said that the stream's `backend`
//! field must also be NULL or a live backend. Since commit 9fe83bce6, the safe
//! `OdbStreamRef::backend` lends that field as an `OdbBackendRef`, and its safe getters read
//! through it. Run under Miri, which reports the read of the freed backend.
//!
//! The same applies to `OdbWritepackRef::from_ptr` and `OdbWritepackRef::backend`.

use ffibox::CRefSeam;
use libgit2::ffi;
use libgit2::libgit2::src::libgit2::odb::OdbStreamRef;

fn main() {
    // A backend that made the stream, then went away. The stream is still a live,
    // initialised `git_odb_stream` in Rust storage.
    let gone = Box::into_raw(Box::new(ffi::git_odb_backend {
        version: ffi::GIT_ODB_BACKEND_VERSION,
        ..ffi::git_odb_backend::default()
    }));
    // SAFETY: `gone` came from `Box::into_raw` just above and is reclaimed once.
    drop(unsafe { Box::from_raw(gone) });
    let mut stream = ffi::git_odb_stream {
        backend: gone,
        ..ffi::git_odb_stream::default()
    };
    // SAFETY: `stream` is a live, initialised `git_odb_stream` in this frame. It outlives
    // the handle, and nothing writes it while the handle lives. This is everything
    // `from_ptr`'s `# Safety` section asks for.
    let handle = unsafe { OdbStreamRef::from_ptr(&raw mut stream) }.unwrap();
    // Safe code from here on: the getter reads `version` through the freed backend.
    let version = handle.backend().map(|backend| backend.version());
    println!("read version {version:?} from a freed backend");
}
