//! `Diff::generate_with_options`, called within its stated `# Safety` contract, lets
//! libgit2 read one byte past an empty pathspec pattern placed beside a non-empty one
//! (AddressSanitizer: heap-buffer-overflow).
//!
//! Every generator copies the options into the diff (`diff_generated_apply_options`,
//! `src/libgit2/diff_generate.c:498`) and compiles `options.pathspec` with
//! `git_pathspec__vinit`. Once one pattern is not empty that parses every pattern, and
//! `parse_optimized_patterns` (`src/libgit2/attr_file.c:842`) tests `pattern[1]` before
//! `pattern[0]`: for `""` that is the byte after the terminator. The contract asked
//! only for a readable array of NUL-terminated strings.
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
//! under `git_diff_tree_to_tree` on the affected commit; prints `refused: ...` and exits
//! 0 once the wrapper refuses such a pathspec before libgit2 parses it.

use core::ffi::c_char;
use std::ffi::CString;

use libgit2::ffi;
use libgit2::libgit2::diff::diff::{Diff, DiffInputs};
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let dir = std::env::temp_dir().join(format!(
        "crustify-diff-options-overread-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    {
        let repo = Repository::init(&CString::new(dir.to_str().unwrap()).unwrap(), false).unwrap();
        // Heap strings of exactly two and one bytes, so the byte after "" is a redzone.
        let patterns = [CString::new("a").unwrap(), CString::new("").unwrap()];
        let mut strings: Vec<*mut c_char> =
            patterns.iter().map(|p| p.as_ptr().cast_mut()).collect();
        let mut opts = ffi::git_diff_options::default();
        // SAFETY: writable storage of the right type; a version this libgit2 supports.
        let rc = unsafe { ffi::git_diff_options_init(&mut opts, ffi::GIT_DIFF_OPTIONS_VERSION) };
        assert_eq!(rc, 0);
        opts.pathspec = ffi::git_strarray {
            strings: strings.as_mut_ptr(),
            count: strings.len(),
        };
        // SAFETY: the contract as stated: `options.version` is GIT_DIFF_OPTIONS_VERSION; the
        // pathspec is a readable array of its stated count (2) of NUL-terminated strings,
        // live (with `opts`) beyond the result; no callbacks or payload; no input trees, so
        // nothing of another object-id type.
        let result = unsafe {
            Diff::generate_with_options(
                repo.as_ref(),
                DiffInputs::Trees {
                    old: None,
                    new: None,
                },
                &opts,
            )
        };
        match result {
            Ok(diff) => println!("generated {} deltas", diff.as_ref().len()),
            Err(error) => println!("refused: {error}"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
