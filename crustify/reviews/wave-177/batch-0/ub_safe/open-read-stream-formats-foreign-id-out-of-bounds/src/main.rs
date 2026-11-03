//! Opening a read stream for a SHA-256 id on a SHA-1 odb overflows the heap
//! (AddressSanitizer).
//!
//! `OdbRef::open_read_stream` passes its id to `git_odb_open_rstream`, which hands it to
//! each backend's `readstream` member without comparing the id's type with the odb's.
//! The loose backend's member (`loose_backend__readstream`) calls `locate_object` →
//! `object_file_name`. That function sizes the path for the odb's id type (41 hex digits
//! and a `/` for SHA-1), but `git_oid_pathfmt` writes as many digits as the id's own type
//! has: 65 for SHA-256. When the objects directory's path is short enough that the
//! buffer's growth slack cannot absorb the extra 24 bytes (any path under ~80 bytes,
//! such as one in `/tmp`), the write runs past the end of the buffer.
//!
//! Build and run against the `asan-ubsan` variant, from this directory, with
//! `<repo>` the repository's root:
//!
//! ```sh
//! CRUSTIFY_BUILD_VARIANT=asan-ubsan \
//! CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Zsanitizer=address \
//! ASAN_OPTIONS=detect_leaks=1:abort_on_error=1 \
//! LD_LIBRARY_PATH=<repo>/crustify/builds/asan-ubsan \
//! cargo +nightly-2026-10-08 run --target x86_64-unknown-linux-gnu
//! ```
//!
//! On the affected commit it aborts with `ERROR: AddressSanitizer: heap-buffer-overflow`
//! (a write in `git_oid_pathfmt` under `object_file_name`). Once the wrapper refuses an
//! id whose type differs from the odb's, it prints the refusal and exits 0.
#![forbid(unsafe_code)]

use std::ffi::CString;

use libgit2::libgit2::src::libgit2::odb::Odb;
use libgit2::libgit2::src::libgit2::oid::{Oid, OidType};

fn main() {
    // A short objects directory path: `/tmp/ors-<pid>`.
    let dir = std::env::temp_dir().join(format!("ors-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = CString::new(dir.to_str().unwrap()).unwrap();

    // A SHA-1 odb over the directory: its loose backend streams reads.
    let odb = Odb::open(&path).unwrap();
    let wide = Oid::from_raw(&[0xab; 32], OidType::Sha256).unwrap();
    let opened = odb.as_ref().open_read_stream(wide.as_ref());
    println!("refused: {:?}", opened.err());

    drop(odb);
    std::fs::remove_dir_all(&dir).unwrap();
}
