//! Safe wrappers over `include/git2/message.h`.
//!
//! # Cleaning up messages
//!
//! [`prettify`] cleans up a commit message as `git commit` does (git's `stripspace`),
//! into a fresh [`BufVal`] of libgit2's allocator.
//!
//! # Trailers
//!
//! A [`MessageTrailer`] is a `git_message_trailer`: one `key: value` pair of the
//! trailer block that ends a commit message (`Signed-off-by: A U Thor <a@example.com>`).
//! It owns nothing. Its only producer, `git_message_trailers`, copies the message's
//! trailer block into one buffer, cuts it into NUL-terminated keys and values in place,
//! and points each trailer's `key` and `value` into it; the
//! `git_message_trailer_array` it fills owns that block and the buffer of trailers, and
//! `git_message_trailer_array_free` frees both, never a string on its own. Neither
//! pointer is ever NULL in a trailer libgit2 emits, nothing writes through them, and no
//! routine takes a trailer back.
//!
//! So a trailer is a pair of borrowed strings, and Rust spells the borrow: a
//! [`MessageTrailer<'t>`] holds its key and value for `'t`, as a `(&'t CStr, &'t CStr)`
//! would, and keeps the C layout, so a run of them is the `trailers` buffer of a
//! `git_message_trailer_array`. Every handle upholds one invariant, which the safe API
//! keeps and [`MessageTrailer::from_raw`] or a crate-internal `from_ptr` over a
//! C-owned trailer must vouch for: `key` and `value` each address a NUL-terminated
//! string that stays live and unchanged for `'t`. Both are read as [`CStr`]s: a key is
//! ASCII letters, digits and `-`, but a value is whatever bytes the message held.
//!
//! The exclusive [`MessageTrailerMut`] is invariant in `'t`, so it only stores strings
//! that live as long as the trailer says. The wrapper of `git_message_trailer_array`
//! must therefore lend the trailers it owns, whose strings live in its own block, at a
//! `'t` no longer than its shared borrow, and give out no [`MessageTrailerMut`] of them:
//! a string stored through one would have to outlive the array, which a borrow of the
//! array cannot promise.
//!
//! # Trailer arrays
//!
//! A [`MessageTrailerArray`] is a `git_message_trailer_array`, which
//! [`MessageTrailerArray::parse`] fills from a message through `git_message_trailers`:
//! `_trailer_block`, the message's trailer block copied into one buffer of libgit2's
//! allocator, and cut into keys and values in place; `trailers`, a buffer of `count`
//! trailers pointing into that block. libgit2 never allocates the struct itself: the
//! caller holds it by value, `git_message_trailers` writes all three fields on success
//! (without freeing what they held) and leaves them untouched on failure, and
//! `git_message_trailer_array_free` frees both buffers, neither zeroing the struct nor
//! freeing it. So Rust holds it by value too, as a [`MessageTrailerArrayVal`], built
//! empty by [`MessageTrailerArray::new`] for libgit2 to fill, and disposed on drop.
//!
//! Every handle upholds one invariant, which the safe API keeps and
//! [`MessageTrailerArray::from_raw`] or a crate-internal `from_ptr` over a C-owned
//! struct must vouch for: `_trailer_block` is NULL or a live buffer of libgit2's
//! allocator that the array owns; `trailers` is NULL or a live buffer of libgit2's
//! allocator, owned by the array, holding at least `count` trailers whose `key` and
//! `value` address NUL-terminated strings that live, unchanged, as long as the array
//! owns that buffer (libgit2's live in the block). A NULL `trailers` reads as no
//! trailer, whatever `count` says. libgit2 leaves `trailers` NULL and `count` 0 when a
//! message has no trailer, and the block allocated even then.
//!
//! The block and the trailers are one resource: the trailers borrow the block, and
//! `count` sizes the trailers. So [`MessageTrailerArrayRef::trailers`] lends them at
//! the array's shared borrow, as the shared handles they are (see above), and
//! [`MessageTrailerArrayMut`] moves the three fields only together: it takes them out
//! into an owner of their own, leaving the empty array
//! ([`take`](MessageTrailerArrayMut::take)), or disposes them and moves another
//! array's in ([`set`](MessageTrailerArrayMut::set),
//! [`clear`](MessageTrailerArrayMut::clear)). Nothing writes a field alone, and
//! nothing writes the buffers once filled.
//!
//! An empty array (both buffers NULL) never reaches libgit2: nothing allocated it, so
//! nothing has initialised the library on its behalf, and even `git__free(NULL)` reads
//! the allocator a concurrent first `git_libgit2_init` writes. Every other array's
//! buffers came from libgit2's allocator, whose allocation initialised the library
//! first.
//!
//! An array is `Send` and `Sync`, as a `Vec` of strings is: it owns its two buffers,
//! which any thread may free into libgit2's allocator, keeps no thread state and no
//! pointer to anything else, and its shared handle only reads.
//!
//! libgit2 defect, out of the wrapper's reach: when growing the trailer buffer fails,
//! `git_message_trailers` writes through the NULL `git_array_alloc` returns, so it
//! crashes on allocation failure instead of failing.

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
