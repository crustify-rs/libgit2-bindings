//! The message cleanup of `src/libgit2/message.c`: [`prettify`], which cleans up a commit
//! message as git does. The trailers of a message are [`trailer`](super::trailer)'s.

use core::ffi::CStr;

use crate::ffi;
use crate::libgit2::src::libgit2::buf::{Buf, BufVal};
use crate::libgit2::src::util::errors::{ErrorBuf, last, runtime_pinned};

/// `message` cleaned up as git cleans up a commit message: trailing whitespace is
/// removed from every line, runs of blank lines are collapsed into one, blank lines at
/// the start and end are removed, and the result ends with a newline unless it is empty.
/// With a `comment` byte, the lines that start with it are removed first (git's
/// `--strip-comments`, `#` by default there); a comment line is any line whose first
/// byte is the comment byte, whitespace before it making it a line like any other.
///
/// Pins the runtime first.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::src::libgit2::message::prettify;
///
/// let message = prettify(c"\n\nsubject  \n\n\n# comment\nbody\t\n\n", Some(b'#')).unwrap();
/// assert_eq!(message.as_ref().as_bytes(), b"subject\n\nbody\n");
/// let kept = prettify(c"subject\n# comment", None).unwrap();
/// assert_eq!(kept.as_ref().as_bytes(), b"subject\n# comment\n");
/// ```
///
/// Wraps: `git_message_prettify`
///
/// # Errors
///
/// Fails, with libgit2's error, when the result cannot be allocated or libgit2 cannot be
/// initialised.
pub fn prettify(message: &CStr, comment: Option<u8>) -> Result<BufVal, ErrorBuf> {
    if !runtime_pinned() {
        return Err(last());
    }
    let mut out = Buf::new();
    let buf = out.as_mut().as_mut_ptr();
    let (strip, comment) = match comment {
        Some(c) => (1, c.cast_signed()),
        None => (0, 0),
    };
    // SAFETY: `buf` addresses the fresh, empty buffer `out` holds, which libgit2 fills
    // (`git_buf_fromstr`) or leaves empty; `message` is NUL-terminated and read for the
    // call only. The runtime is pinned, so allocations and errors reach live state.
    let rc = unsafe { ffi::git_message_prettify(buf, message.as_ptr(), strip, comment) };
    if rc < 0 { Err(last()) } else { Ok(out) }
}
