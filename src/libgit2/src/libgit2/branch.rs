//! Safe wrappers over `include/git2/types.h`.
//!
//! The header's enums travel by value, and through out-parameters libgit2 or a user
//! callback fills, so a value read back from C may be one the header does not name
//! for the purpose at hand: a lookup filter or an error answer where an object's type
//! is expected, `-1` from `git_reference_type(NULL)`, or a setting libgit2 refuses to
//! write. A raw value is therefore converted fallibly (each enum's `from_raw` and
//! `TryFrom`) and never transmuted.
//!
//! [`ObjectType`] names the four types an object can have, the values
//! `git_object_type_is_valid` accepts. `GIT_OBJECT_ANY` is not one: it is the filter
//! that lookups and peels take to mean "whatever type the object has", spelled `None`
//! by [`ObjectType::raw_or_any`]. `GIT_OBJECT_INVALID` is what routines answer when
//! they have no type (`git_object_string2type` for an unknown name,
//! `git_object_type(NULL)`). libgit2 rejects every other value as an "invalid object
//! type", except `0`: `git_odb_write` reads it as a blob for the object's id, but its
//! loose backend stores the object under an empty type name, so the object cannot be
//! read back; a typed argument cannot carry it.
//!
//! [`ReferenceType`] names the two kinds of reference `git_reference_type` answers
//! for every reference libgit2 builds. `GIT_REFERENCE_INVALID` (a reference without
//! a target, inside a transaction) and `GIT_REFERENCE_ALL` (the union of both kinds,
//! a listing filter no current routine takes) are not kinds a reference has.
//!
//! [`BranchType`] names the three selections every branch routine accepts: local,
//! remote-tracking, or both. Its C enum is flag-shaped; `0` and other bits, which an
//! iterator reads as flags but `git_branch_lookup` asserts on, have no variant.
//!
//! The submodule settings ([`SubmoduleIgnore`], [`SubmoduleRecurse`],
//! [`SubmoduleUpdate`]) are fieldless enums over their C values, converted fallibly
//! from a raw one.
//!
//! A [`Time`] is a `git_time`: the moment a signature records, as seconds since the
//! epoch, the timezone's offset from UTC in minutes, and the sign character the
//! offset was written with. It owns nothing, so it is held by value, copied freely
//! and never released; libgit2 embeds one in every `git_signature` (`when`), whose
//! wrapper hands it out as a [`TimeRef`] / [`TimeMut`] into the signature.
//!
//! Every value is a valid `git_time` to libgit2, and its producers leave more than
//! `'+'` and `'-'` in `sign`: `git_signature_new` derives the sign from the offset
//! (`'-'` when it is negative, `'+'` otherwise, as [`Time::new`] does), but
//! `git_signature_from_buffer` stores the timezone's first character verbatim, so a
//! malformed timezone such as `x0130` gives offset 0 and sign `'x'`, a missing or
//! out-of-range one (`+1500`) leaves both 0, and `+-130` gives offset -90 with sign
//! `'+'`. The sign only matters for an offset of 0: libgit2 writes a signature's
//! zone as negative when the offset is negative or the sign is `'-'`, which is how a
//! questionable `-0000` survives a round trip. So the accessors take and return any
//! byte, and the offset and sign are independent.
//!
//! libgit2 accepts any value in a signature, but its one writer of a time
//! (`git_signature__writebuf`, behind commit, tag and reflog creation) is narrower than
//! the type: it prints the seconds through a 32-bit unsigned cast, so a time before
//! the epoch or past 2106 is written wrapped, and it negates a negative offset as an
//! `int`, which overflows for `i32::MIN` (undefined behaviour in C, an abort under
//! `UBSan`). The wrappers of those routines must refuse such a time or document the
//! wrap; the type itself carries what libgit2 stores.
//!
//! Times compare as libgit2 compares signatures' times: equal when all three fields
//! are, whatever the struct's padding holds.
//!
//! A [`FileMode`] is a tree entry's `git_filemode_t`. A parsed tree keeps whatever
//! 16-bit mode its object spells, so a raw mode is converted fallibly
//! ([`FileMode::from_raw`]) and never transmuted.
//!
//! # Write streams
//!
//! A `git_writestream` is a vtable header: libgit2 embeds it as the first member of a
//! concrete stream (a blob writer, a filter's buffered stream, the checkout writer), and
//! each of its three members takes the header back and casts it to that container. A
//! header is therefore only valid where its producer put it: it is never held by value,
//! and every handle designates one in place.
//!
//! A stream is written any number of times, then closed once, then freed:
//!
//! - [`WriteStreamMut::write`] calls the `write` member;
//! - closing ends the stream: libgit2's own streams accept nothing but `free` afterwards (a blob
//!   stream's next write or commit calls a NULL function, since closing zeroes its file buffer; the
//!   checkout writer writes to a closed file descriptor), so [`WriteStream::close`] consumes the
//!   owner;
//! - the `free` member releases the whole container. [`WriteStreamFree`] calls it, so one owner,
//!   [`WriteStreamBox`], holds any stream, C's or Rust's.
//!
//! [`RustWriteStream`] implements a stream in Rust, over a [`StreamWriter`]: its members
//! are `extern "C"` trampolines into the writer. It lends a [`WriteStreamMut`] to code
//! that writes into it (libgit2's `git_filter_list_stream_*` take one as their target,
//! write into it and close it, but never free it), or becomes a [`WriteStreamBox`] to
//! hand over to libgit2 (a filter's `stream` callback returns one, which libgit2 frees).
//!
//! # Signatures
//!
//! A `git_signature` is a name, an email and a [`Time`]: who did something, and when.
//! libgit2 hands signatures out in two ways:
//!
//! - **Owned.** `git_signature_new`, `git_signature_now`, `git_signature_from_buffer` and
//!   `git_signature_dup` allocate the struct and copies of both strings from libgit2's allocator,
//!   and `git_signature_free` releases all three. Rust holds such a signature as a [`SignatureBox`]
//!   ([`Signature::new`], [`Signature::now`], [`Signature::parse`], `Clone`). Its strings move in
//!   and out as [`OdbDataStr`]s, which the same `git__free` releases.
//! - **Borrowed.** Commits, tags, notes, reflog entries and blame hunks own the signatures they
//!   record and lend them as `const git_signature *`, valid while their holder lives: a
//!   [`SignatureRef`] tied to that holder.
//!
//! Every `const git_signature *` libgit2 takes (commit, tag, note, reflog and stash
//! creation, rebase commits) is only read, and what libgit2 keeps of it is copied. Rust
//! builds one over borrowed strings as a [`SignatureView`], as C code builds one on the
//! stack.
//!
//! Every handle upholds one invariant, which the safe constructors and setters keep and a
//! crate-internal `from_ptr` must vouch for: `name` and `email` are each NULL or a
//! NUL-terminated string that outlives the handle, and behind a [`SignatureMut`] each is
//! NULL or an allocation of libgit2's allocator that the signature owns. Rust never
//! writes NULL. libgit2's parser leaves it when it runs out of memory copying a string,
//! and still reports success, in `git_signature_from_buffer` and in the commits, tags and
//! reflogs it parses: [`Signature::parse`] refuses such a signature, the getters read the
//! lost string as `None`, and copies refuse it, since `git_signature_dup` would
//! `strdup(NULL)`. Wrappers that pass a borrowed signature to libgit2's writers must
//! refuse it too: they print both strings with `%s`.
//!
//! libgit2 never checks a signature it is given. [`Signature::new`] and [`Signature::now`]
//! trim the strings and refuse angle brackets and empty ones, but a [`SignatureView`] and
//! the setters take their strings as they are, and a name or email holding a bracket or a
//! newline is written into objects git cannot parse back.
//!
//! `git_signature_default` and `git_signature_default_from_env` read the identity from a
//! repository's configuration, so they are wrapped in
//! [`object::signature`](crate::libgit2::src::libgit2::signature), as [`Signature::default`]
//! and [`Signature::default_from_env`].

use core::fmt;
use core::hash::Hash;
use core::ops::BitOr;

use crate::ffi;

/// Which branches a branch routine means: local ones (under `refs/heads/`), remote-tracking
/// ones (under `refs/remotes/`), or both.
///
/// The C enum is flag-shaped, `GIT_BRANCH_ALL` being `GIT_BRANCH_LOCAL | GIT_BRANCH_REMOTE`,
/// and libgit2 reads it both ways: `git_branch_iterator_new` tests each bit, so `0` lists
/// nothing and other bits are ignored, while `git_branch_lookup` accepts the three named
/// values only and fails an internal assertion on any other (an error in release builds, an
/// abort in debug builds). `git_branch_next` reports a branch as [`Local`](Self::Local) or
/// [`Remote`](Self::Remote), never [`All`](Self::All). The three variants are the values
/// every routine accepts; [`from_raw`](Self::from_raw) gives `None` for the rest.
///
/// Wraps: `git_branch_t`
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum BranchType {
    /// Local branches, `refs/heads/*`.
    Local = ffi::GIT_BRANCH_LOCAL,
    /// Remote-tracking branches, `refs/remotes/*`.
    Remote = ffi::GIT_BRANCH_REMOTE,
    /// Both kinds.
    All = ffi::GIT_BRANCH_ALL,
}

impl BranchType {
    /// The variant whose C value is `raw`; `None` for any value libgit2's header does not
    /// name, `0` included.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_branch_t) -> Option<Self> {
        match raw {
            ffi::GIT_BRANCH_LOCAL => Some(Self::Local),
            ffi::GIT_BRANCH_REMOTE => Some(Self::Remote),
            ffi::GIT_BRANCH_ALL => Some(Self::All),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_branch_t {
        self as ffi::git_branch_t
    }

    /// Whether every branch `other` means is also one `self` means, as
    /// `git_branch_next` tests a listed branch's kind against the iterator's flags:
    /// [`All`](Self::All) contains every kind, and each kind contains itself.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.raw() & other.raw() == other.raw()
    }
}

/// The union of two selections: `Local | Remote` is [`All`](BranchType::All).
impl BitOr for BranchType {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        match (self, rhs) {
            (Self::Local, Self::Local) => Self::Local,
            (Self::Remote, Self::Remote) => Self::Remote,
            _ => Self::All,
        }
    }
}

impl TryFrom<ffi::git_branch_t> for BranchType {
    type Error = UnnamedBranchType;

    fn try_from(raw: ffi::git_branch_t) -> Result<Self, UnnamedBranchType> {
        Self::from_raw(raw).ok_or(UnnamedBranchType(raw))
    }
}

impl From<BranchType> for ffi::git_branch_t {
    fn from(value: BranchType) -> ffi::git_branch_t {
        value.raw()
    }
}

/// A branch type that libgit2's header does not name: `0`, which a branch iterator reads
/// as listing nothing, or a value with bits beyond `GIT_BRANCH_ALL`. [`raw`](Self::raw)
/// has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedBranchType(ffi::git_branch_t);

impl UnnamedBranchType {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_branch_t {
        self.0
    }
}

impl fmt::Display for UnnamedBranchType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unnamed branch type {}", self.0)
    }
}

impl std::error::Error for UnnamedBranchType {}
