//! Inserting a SHA-256 id into a SHA-1 repository's tree builder overflows the heap
//! (AddressSanitizer).
//!
//! `TreeBuilderMut::insert` hands its id to `git_treebuilder_insert`, which, for any
//! file mode but a submodule's, checks that the object exists with
//! `git_object__is_valid` → `git_odb_read_header`. The loose backend formats the id
//! into a path it sized for the repository's id type (`object_file_name`: 41 hex
//! digits and a `/` for SHA-1), but `git_oid_pathfmt` writes as many digits as the
//! id's own type has: 65 for SHA-256, past the end of the buffer when the objects
//! directory's path is short enough that the buffer's growth slack does not absorb
//! the extra 24 bytes (any path under ~80 bytes, such as one in `/tmp`).
//!
//! Build and run against the `asan-ubsan` variant, from this directory:
//!
//! ```sh
//! CRUSTIFY_BUILD_VARIANT=asan-ubsan \
//! CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS=-Zsanitizer=address \
//! ASAN_OPTIONS=detect_leaks=1:abort_on_error=1 \
//! LD_LIBRARY_PATH=/target/crustify/builds/asan-ubsan \
//! cargo +nightly run --target x86_64-unknown-linux-gnu
//! ```
//!
//! Aborts with `ERROR: AddressSanitizer: heap-buffer-overflow` (write in
//! `git_oid_pathfmt` under `object_file_name`) on the affected commit; prints the
//! refusal and exits 0 once the wrapper refuses an id of another type than the
//! repository's.
#![forbid(unsafe_code)]

use std::ffi::CString;

use libgit2::libgit2::core::types::FileMode;
use libgit2::libgit2::object::tree::TreeBuilder;
use libgit2::libgit2::oid::oid::{Oid, OidType};
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    // A short objects directory path: `/tmp/tbi-<pid>/objects/`.
    let dir = std::env::temp_dir().join(format!("tbi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = CString::new(dir.to_str().unwrap()).unwrap();
    let repo = Repository::init(&path, true).unwrap();
    assert_eq!(repo.as_ref().oid_type(), OidType::Sha1);

    // Any non-zero SHA-256 id.
    let wide = Oid::from_raw(&[0xab; 32], OidType::Sha256).unwrap();
    let mut builder = TreeBuilder::new(repo.as_ref()).unwrap();
    let result = builder.as_mut().insert(c"file", wide.as_ref(), FileMode::Blob);
    println!("insert returned {result:?}");
    drop(builder);
    drop(repo);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(result.is_err(), "a SHA-256 id was accepted by a SHA-1 repository's builder");
}
