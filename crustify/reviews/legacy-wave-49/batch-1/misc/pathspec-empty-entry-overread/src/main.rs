//! Compiling a pathspec that holds an empty pattern beside a non-empty one reads one
//! byte past the empty string (AddressSanitizer: heap-buffer-overflow).
//!
//! `git_pathspec__vinit` parses every entry once the array is not all empty, and
//! `parse_optimized_patterns` (`src/libgit2/attr_file.c:842`) tests `pattern[1]`
//! before `pattern[0]`: for `""` that is the byte after the terminator.
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
//! under `git_pathspec_new` on the affected commit; exits 0 once `Pathspec::new`
//! refuses such an array before libgit2 parses it.
#![forbid(unsafe_code)]

use std::ffi::CString;

use libgit2::libgit2::core::strarray::StrArrayView;
use libgit2::libgit2::index::pathspec::Pathspec;

fn main() {
    // A heap allocation of exactly one byte, so the byte after it is a redzone.
    let patterns = [CString::new("a").unwrap(), CString::new("").unwrap()];
    let view = StrArrayView::new(&patterns);
    match Pathspec::new(view.as_ref()) {
        Ok(_) => println!("compiled"),
        Err(error) => println!("refused: {:?}", error),
    }
}
