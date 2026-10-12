//! Writing a repository's index that holds an entry with an empty path beside another
//! entry, both racily clean, makes libgit2 read one byte past the empty path
//! (AddressSanitizer: heap-buffer-overflow).
//!
//! `git_index_write` first calls `truncate_racily_clean` (`src/libgit2/index.c:770`),
//! which hands the racy entries' paths to `git_diff_index_to_workdir` as its pathspec.
//! `git_pathspec__vinit` parses every pattern once one is non-empty, and
//! `parse_optimized_patterns` (`src/libgit2/attr_file.c:842`) tests `pattern[1]` first:
//! for the entry `""` that is the byte after its terminator, at the end of the entry's
//! allocation. libgit2 reads such an entry from an index file without complaint.
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
//! Aborts with `heap-buffer-overflow ... READ of size 1` in `parse_optimized_patterns`
//! under `git_index_write` on the affected commit; exits 0 once `IndexRef::write`
//! refuses such an index before libgit2 scans it.
#![forbid(unsafe_code)]

use std::ffi::CString;

use libgit2::libgit2::repository::repository::Repository;

/// A version 2 index entry at `path`, modified in 2038 (so newer than the index file:
/// racily clean), with a non-zero id.
fn entry(path: &str) -> Vec<u8> {
    let mut entry = Vec::new();
    // ctime, mtime (seconds, nanoseconds), dev, ino, mode, uid, gid, size.
    for field in [0u32, 0, 0x7fff_ffff, 0, 0, 0, 0o100644, 0, 0, 0] {
        entry.extend_from_slice(&field.to_be_bytes());
    }
    entry.extend_from_slice(&[0x11; 20]);
    entry.extend_from_slice(&u16::try_from(path.len()).unwrap().to_be_bytes());
    entry.extend_from_slice(path.as_bytes());
    entry.push(0);
    while entry.len() % 8 != 0 {
        entry.push(0);
    }
    entry
}

fn main() {
    let dir = std::env::temp_dir().join(format!("index-write-overread-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let repo = Repository::init(&CString::new(dir.to_str().unwrap()).unwrap(), false).unwrap();

    // Two entries, "" and "a"; a zero checksum, which libgit2 accepts (`index.skipHash`).
    let mut file = b"DIRC".to_vec();
    file.extend_from_slice(&2u32.to_be_bytes());
    file.extend_from_slice(&2u32.to_be_bytes());
    file.extend(entry(""));
    file.extend(entry("a"));
    file.extend_from_slice(&[0; 20]);
    std::fs::write(dir.join(".git/index"), &file).unwrap();

    let index = repo.as_ref().index().unwrap();
    println!("entries: {}", index.as_ref().entry_count());
    match index.as_ref().write() {
        Ok(()) => println!("written"),
        Err(error) => println!("refused: {error:?}"),
    }
    drop((index, repo));
    let _ = std::fs::remove_dir_all(&dir);
}
