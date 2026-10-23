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
//! [`object::signature`](crate::libgit2::object::signature), as [`Signature::default`]
//! and [`Signature::default_from_env`].

use core::ffi::{CStr, c_char, c_int};
use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::mem::ManuallyDrop;
use core::ops::BitOr;
use core::ptr::{self, NonNull, addr_of, addr_of_mut};
use std::ffi::CString;

use ffibox::{CBox, CZeroable, define_ctype, impl_cdrop, impl_cdupclone};

use crate::ffi;
use crate::libgit2::raw::OdbDataStr;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::{runtime_pinned, set, set_oom};

/// The type of a Git object: what its content is, which fixes how libgit2 parses it.
///
/// libgit2 answers `GIT_OBJECT_INVALID` where it has no type, and its lookups take
/// `GIT_OBJECT_ANY` to accept any type ([`raw_or_any`](Self::raw_or_any));
/// [`from_raw`](Self::from_raw) gives `None` for both, and for every value
/// `git_object_type_is_valid` refuses.
///
/// Wraps: `git_object_t`
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(i32)]
#[non_exhaustive]
pub enum ObjectType {
    /// A commit object.
    Commit = ffi::GIT_OBJECT_COMMIT,
    /// A tree (directory listing) object.
    Tree = ffi::GIT_OBJECT_TREE,
    /// A file revision object.
    Blob = ffi::GIT_OBJECT_BLOB,
    /// An annotated tag object.
    Tag = ffi::GIT_OBJECT_TAG,
}

impl ObjectType {
    /// The variant whose C value is `raw`; `None` for `GIT_OBJECT_ANY`,
    /// `GIT_OBJECT_INVALID` and every value the header does not name.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_object_t) -> Option<Self> {
        match raw {
            ffi::GIT_OBJECT_COMMIT => Some(Self::Commit),
            ffi::GIT_OBJECT_TREE => Some(Self::Tree),
            ffi::GIT_OBJECT_BLOB => Some(Self::Blob),
            ffi::GIT_OBJECT_TAG => Some(Self::Tag),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_object_t {
        self as ffi::git_object_t
    }

    /// The C value of a type filter, as lookups and peels take it: `kind`'s value, or
    /// `GIT_OBJECT_ANY` for `None`, which accepts an object of any type.
    #[must_use]
    pub const fn raw_or_any(kind: Option<Self>) -> ffi::git_object_t {
        match kind {
            Some(kind) => kind.raw(),
            None => ffi::GIT_OBJECT_ANY,
        }
    }
}

impl TryFrom<ffi::git_object_t> for ObjectType {
    type Error = NotAnObjectType;

    fn try_from(raw: ffi::git_object_t) -> Result<Self, NotAnObjectType> {
        Self::from_raw(raw).ok_or(NotAnObjectType(raw))
    }
}

impl From<ObjectType> for ffi::git_object_t {
    fn from(value: ObjectType) -> ffi::git_object_t {
        value.raw()
    }
}

/// A `git_object_t` value that is no object's type: `GIT_OBJECT_ANY`,
/// `GIT_OBJECT_INVALID`, or a value the header does not name. [`raw`](Self::raw) has
/// it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotAnObjectType(ffi::git_object_t);

impl NotAnObjectType {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_object_t {
        self.0
    }
}

impl fmt::Display for NotAnObjectType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "libgit2 value {} is not an object type", self.0)
    }
}

impl std::error::Error for NotAnObjectType {}

/// The kind of a Git reference: whether it points at an object id or at another
/// reference.
///
/// [`from_raw`](Self::from_raw) gives `None` for `GIT_REFERENCE_INVALID`,
/// `GIT_REFERENCE_ALL` and every value the header does not name, such as the `-1`
/// that `git_reference_type(NULL)` returns in a release build.
///
/// Wraps: `git_reference_t`
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum ReferenceType {
    /// A reference that points at an object id.
    Direct = ffi::GIT_REFERENCE_DIRECT,
    /// A reference that points at another reference.
    Symbolic = ffi::GIT_REFERENCE_SYMBOLIC,
}

impl ReferenceType {
    /// The variant whose C value is `raw`; `None` for `GIT_REFERENCE_INVALID`,
    /// `GIT_REFERENCE_ALL` and every value the header does not name.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_reference_t) -> Option<Self> {
        match raw {
            ffi::GIT_REFERENCE_DIRECT => Some(Self::Direct),
            ffi::GIT_REFERENCE_SYMBOLIC => Some(Self::Symbolic),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions return it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_reference_t {
        self as ffi::git_reference_t
    }
}

impl TryFrom<ffi::git_reference_t> for ReferenceType {
    type Error = NotAReferenceType;

    fn try_from(raw: ffi::git_reference_t) -> Result<Self, NotAReferenceType> {
        Self::from_raw(raw).ok_or(NotAReferenceType(raw))
    }
}

impl From<ReferenceType> for ffi::git_reference_t {
    fn from(value: ReferenceType) -> ffi::git_reference_t {
        value.raw()
    }
}

/// A `git_reference_t` value that is no reference's kind: `GIT_REFERENCE_INVALID`,
/// `GIT_REFERENCE_ALL`, or a value the header does not name. [`raw`](Self::raw) has
/// it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotAReferenceType(ffi::git_reference_t);

impl NotAReferenceType {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_reference_t {
        self.0
    }
}

impl fmt::Display for NotAReferenceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "libgit2 value {} is not a reference type", self.0)
    }
}

impl std::error::Error for NotAReferenceType {}

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

/// Whether `git fetch` recurses into a submodule: the value of its
/// `submodule.<name>.fetchRecurseSubmodules` setting.
///
/// libgit2 reads `false` as [`No`](Self::No), `true` as [`Yes`](Self::Yes) and
/// `on-demand` as [`OnDemand`](Self::OnDemand), and writes the latter two back that
/// way; it writes [`No`](Self::No), also what an unset setting reads as, by removing
/// the setting. `git_submodule_set_fetch_recurse_submodules` refuses any value
/// without a variant ("invalid value for fetchRecurseSubmodules"), and
/// [`from_raw`](Self::from_raw) gives `None` for it.
///
/// Wraps: `git_submodule_recurse_t`
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum SubmoduleRecurse {
    /// Do not recurse into the submodule.
    No = ffi::GIT_SUBMODULE_RECURSE_NO,
    /// Recurse into the submodule.
    Yes = ffi::GIT_SUBMODULE_RECURSE_YES,
    /// Recurse into the submodule only when the commit the superproject records is
    /// not already in its local clone.
    OnDemand = ffi::GIT_SUBMODULE_RECURSE_ONDEMAND,
}

impl SubmoduleRecurse {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_submodule_recurse_t) -> Option<Self> {
        match raw {
            ffi::GIT_SUBMODULE_RECURSE_NO => Some(Self::No),
            ffi::GIT_SUBMODULE_RECURSE_YES => Some(Self::Yes),
            ffi::GIT_SUBMODULE_RECURSE_ONDEMAND => Some(Self::OnDemand),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_submodule_recurse_t {
        self as ffi::git_submodule_recurse_t
    }
}

/// What libgit2 reports for a submodule whose configuration does not set
/// `fetchRecurseSubmodules`: [`No`](Self::No).
impl Default for SubmoduleRecurse {
    fn default() -> Self {
        Self::No
    }
}

impl TryFrom<ffi::git_submodule_recurse_t> for SubmoduleRecurse {
    type Error = UnnamedSubmoduleRecurse;

    fn try_from(raw: ffi::git_submodule_recurse_t) -> Result<Self, UnnamedSubmoduleRecurse> {
        Self::from_raw(raw).ok_or(UnnamedSubmoduleRecurse(raw))
    }
}

impl From<SubmoduleRecurse> for ffi::git_submodule_recurse_t {
    fn from(value: SubmoduleRecurse) -> ffi::git_submodule_recurse_t {
        value.raw()
    }
}

/// A submodule recursion setting that this version of libgit2's header does not
/// name, which libgit2 refuses to write. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedSubmoduleRecurse(ffi::git_submodule_recurse_t);

impl UnnamedSubmoduleRecurse {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_submodule_recurse_t {
        self.0
    }
}

impl fmt::Display for UnnamedSubmoduleRecurse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 submodule recurse setting {} is not named by git2/types.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedSubmoduleRecurse {}

/// How `git submodule update` updates a submodule: the value of its
/// `submodule.<name>.update` setting.
///
/// libgit2 reads `checkout`, `rebase`, `merge` and `none` as the variants of those
/// names, `true` as [`Checkout`](Self::Checkout) and `false` as [`None`](Self::None),
/// and writes each variant back by its name.
///
/// The header also names `GIT_SUBMODULE_UPDATE_DEFAULT` (0), "not used except as
/// static initializer": no libgit2 routine produces it,
/// `git_submodule_set_update` refuses it like any other value without a variant
/// ("invalid value for update"), and `git_submodule_update_strategy` would report it
/// as [`Checkout`](Self::Checkout). So it has no variant: [`from_raw`](Self::from_raw)
/// gives `None` for it, and `from_raw(raw).unwrap_or_default()` resolves it as
/// libgit2 does.
///
/// Wraps: `git_submodule_update_t`
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum SubmoduleUpdate {
    /// Check the new commit out as a detached `HEAD` of the submodule.
    Checkout = ffi::GIT_SUBMODULE_UPDATE_CHECKOUT,
    /// Rebase the submodule's current branch onto the new commit.
    Rebase = ffi::GIT_SUBMODULE_UPDATE_REBASE,
    /// Merge the new commit into the submodule's current branch.
    Merge = ffi::GIT_SUBMODULE_UPDATE_MERGE,
    /// Leave the submodule alone, even when the superproject's commit changes.
    None = ffi::GIT_SUBMODULE_UPDATE_NONE,
}

impl SubmoduleUpdate {
    /// The variant whose C value is `raw`; `None` for `GIT_SUBMODULE_UPDATE_DEFAULT`
    /// and for a value this version of libgit2's header does not name.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_submodule_update_t) -> Option<Self> {
        match raw {
            ffi::GIT_SUBMODULE_UPDATE_CHECKOUT => Some(Self::Checkout),
            ffi::GIT_SUBMODULE_UPDATE_REBASE => Some(Self::Rebase),
            ffi::GIT_SUBMODULE_UPDATE_MERGE => Some(Self::Merge),
            ffi::GIT_SUBMODULE_UPDATE_NONE => Some(Self::None),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_submodule_update_t {
        self as ffi::git_submodule_update_t
    }
}

/// What libgit2 reports for a submodule whose configuration does not set `update`,
/// and what it resolves `GIT_SUBMODULE_UPDATE_DEFAULT` to:
/// [`Checkout`](Self::Checkout).
impl Default for SubmoduleUpdate {
    fn default() -> Self {
        Self::Checkout
    }
}

impl TryFrom<ffi::git_submodule_update_t> for SubmoduleUpdate {
    type Error = UnnamedSubmoduleUpdate;

    fn try_from(raw: ffi::git_submodule_update_t) -> Result<Self, UnnamedSubmoduleUpdate> {
        Self::from_raw(raw).ok_or(UnnamedSubmoduleUpdate(raw))
    }
}

impl From<SubmoduleUpdate> for ffi::git_submodule_update_t {
    fn from(value: SubmoduleUpdate) -> ffi::git_submodule_update_t {
        value.raw()
    }
}

/// A submodule update setting without a [`SubmoduleUpdate`] variant:
/// `GIT_SUBMODULE_UPDATE_DEFAULT` (0), or a value this version of libgit2's header
/// does not name. libgit2 refuses to write either. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedSubmoduleUpdate(ffi::git_submodule_update_t);

impl UnnamedSubmoduleUpdate {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_submodule_update_t {
        self.0
    }
}

impl fmt::Display for UnnamedSubmoduleUpdate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 submodule update setting {} has no SubmoduleUpdate variant",
            self.0
        )
    }
}

impl std::error::Error for UnnamedSubmoduleUpdate {}

/// How deeply submodule status looks into a submodule's working directory: the
/// `submodule.<name>.ignore` setting, as `git_submodule_status`, `git_submodule_ignore`
/// and the `ignore_submodules` of diff options read it.
///
/// [`Unspecified`](Self::Unspecified) is a request, not a setting: "whatever the
/// submodule (or, for a diff, `diff.ignoreSubmodules`) is configured with". It is the
/// value libgit2's option initialisers select, hence [`Default`], and the one
/// `git_submodule_set_ignore` refuses ("invalid value for ignore"), as it refuses every
/// value outside `None..=All`. libgit2 reads the rest by equality, so a value without a
/// variant behaves as [`Untracked`](Self::Untracked) in a status, and a diff's `0` (a
/// zeroed options struct) as `Unspecified`; [`from_raw`](Self::from_raw) gives `None`
/// for all of them. libgit2's own routines return only named values.
///
/// Wraps: `git_submodule_ignore_t`
/// Handwritten
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(i32)]
#[non_exhaustive]
pub enum SubmoduleIgnore {
    /// Use the submodule's configuration (`GIT_SUBMODULE_IGNORE_UNSPECIFIED`).
    #[default]
    Unspecified = ffi::GIT_SUBMODULE_IGNORE_UNSPECIFIED,
    /// Any change, untracked files included, makes the submodule dirty; ignored files
    /// are still ignored (`GIT_SUBMODULE_IGNORE_NONE`, config `none`).
    None = ffi::GIT_SUBMODULE_IGNORE_NONE,
    /// Only changes to tracked files, the index or the HEAD commit count
    /// (`GIT_SUBMODULE_IGNORE_UNTRACKED`, config `untracked`).
    Untracked = ffi::GIT_SUBMODULE_IGNORE_UNTRACKED,
    /// The working directory is not scanned: only a moved HEAD counts
    /// (`GIT_SUBMODULE_IGNORE_DIRTY`, config `dirty`).
    Dirty = ffi::GIT_SUBMODULE_IGNORE_DIRTY,
    /// Never dirty: only the submodule's location is reported
    /// (`GIT_SUBMODULE_IGNORE_ALL`, config `all`).
    All = ffi::GIT_SUBMODULE_IGNORE_ALL,
}

impl SubmoduleIgnore {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_submodule_ignore_t) -> Option<Self> {
        match raw {
            ffi::GIT_SUBMODULE_IGNORE_UNSPECIFIED => Some(Self::Unspecified),
            ffi::GIT_SUBMODULE_IGNORE_NONE => Some(Self::None),
            ffi::GIT_SUBMODULE_IGNORE_UNTRACKED => Some(Self::Untracked),
            ffi::GIT_SUBMODULE_IGNORE_DIRTY => Some(Self::Dirty),
            ffi::GIT_SUBMODULE_IGNORE_ALL => Some(Self::All),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_submodule_ignore_t {
        self as ffi::git_submodule_ignore_t
    }
}

impl TryFrom<ffi::git_submodule_ignore_t> for SubmoduleIgnore {
    type Error = UnnamedSubmoduleIgnore;

    fn try_from(raw: ffi::git_submodule_ignore_t) -> Result<Self, UnnamedSubmoduleIgnore> {
        Self::from_raw(raw).ok_or(UnnamedSubmoduleIgnore(raw))
    }
}

impl From<SubmoduleIgnore> for ffi::git_submodule_ignore_t {
    fn from(value: SubmoduleIgnore) -> ffi::git_submodule_ignore_t {
        value.raw()
    }
}

/// A submodule ignore value that this version of libgit2's header does not name: `0`,
/// which a zeroed diff options struct holds, or anything outside `-1` and `1..=4`.
/// [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedSubmoduleIgnore(ffi::git_submodule_ignore_t);

impl UnnamedSubmoduleIgnore {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_submodule_ignore_t {
        self.0
    }
}

impl fmt::Display for UnnamedSubmoduleIgnore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 submodule ignore value {} is not named by git2/types.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedSubmoduleIgnore {}

// A `SubmoduleIgnore` passes where C takes a `git_submodule_ignore_t`.
const _: () = assert!(
    size_of::<SubmoduleIgnore>() == size_of::<ffi::git_submodule_ignore_t>()
        && align_of::<SubmoduleIgnore>() == align_of::<ffi::git_submodule_ignore_t>()
);

/// The mode of a tree entry: the kind of object it names, and for a blob whether it
/// is an executable or a symbolic link (`git_filemode_t`, UNIX modes in octal).
///
/// `git_treebuilder_insert` accepts every variant but [`Unreadable`](Self::Unreadable)
/// and refuses every other value with `GIT_ERROR_TREE` ("failed to insert entry:
/// invalid filemode for file"). A parsed tree, though, stores whatever 16-bit mode
/// its object spells: `git_tree_entry_filemode` normalises it to one of the five
/// insertable variants, while `git_tree_entry_filemode_raw` returns it as is, such as
/// old git's group-writable `0100664`, or `0`, which reads as `Unreadable`.
/// [`from_raw`](Self::from_raw) gives `None` for a value without a variant.
///
/// Wraps: `git_filemode_t`
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum FileMode {
    /// `0000000`: no object. libgit2 gives it to a working-directory file it could
    /// not read; no tree entry can be inserted with it.
    Unreadable = ffi::GIT_FILEMODE_UNREADABLE,
    /// `0040000`: a subtree.
    Tree = ffi::GIT_FILEMODE_TREE,
    /// `0100644`: a regular file's blob.
    Blob = ffi::GIT_FILEMODE_BLOB,
    /// `0100755`: an executable file's blob.
    BlobExecutable = ffi::GIT_FILEMODE_BLOB_EXECUTABLE,
    /// `0120000`: a blob holding a symbolic link's target.
    Link = ffi::GIT_FILEMODE_LINK,
    /// `0160000`: a submodule's commit, which the repository need not contain.
    Commit = ffi::GIT_FILEMODE_COMMIT,
}

impl FileMode {
    /// The variant whose C value is `raw`; `None` for a mode this version of
    /// libgit2's header does not name.
    #[must_use]
    pub const fn from_raw(raw: ffi::git_filemode_t) -> Option<Self> {
        match raw {
            ffi::GIT_FILEMODE_UNREADABLE => Some(Self::Unreadable),
            ffi::GIT_FILEMODE_TREE => Some(Self::Tree),
            ffi::GIT_FILEMODE_BLOB => Some(Self::Blob),
            ffi::GIT_FILEMODE_BLOB_EXECUTABLE => Some(Self::BlobExecutable),
            ffi::GIT_FILEMODE_LINK => Some(Self::Link),
            ffi::GIT_FILEMODE_COMMIT => Some(Self::Commit),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
    #[must_use]
    pub const fn raw(self) -> ffi::git_filemode_t {
        self as ffi::git_filemode_t
    }
}

impl TryFrom<ffi::git_filemode_t> for FileMode {
    type Error = UnnamedFileMode;

    fn try_from(raw: ffi::git_filemode_t) -> Result<Self, UnnamedFileMode> {
        Self::from_raw(raw).ok_or(UnnamedFileMode(raw))
    }
}

impl From<FileMode> for ffi::git_filemode_t {
    fn from(value: FileMode) -> ffi::git_filemode_t {
        value.raw()
    }
}

/// A file mode that this version of libgit2's header does not name, as
/// `git_tree_entry_filemode_raw` returns for a tree that spells one;
/// [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedFileMode(ffi::git_filemode_t);

impl UnnamedFileMode {
    /// The value, as it was converted.
    #[must_use]
    pub const fn raw(self) -> ffi::git_filemode_t {
        self.0
    }
}

impl fmt::Display for UnnamedFileMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 file mode {:#o} is not named by git2/types.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedFileMode {}

define_ctype!(
    /// A point in time as a git signature records it (`git_time`): seconds since the
    /// epoch, the timezone offset in minutes, and the offset's sign character.
    ///
    /// A plain value of 16 bytes, resource-free: `Copy`, held inline, never released.
    /// Build one with [`new`](Self::new), or start from
    /// [`Time::zeroed`](ffibox::CZeroable::zeroed) (the epoch, offset 0, sign 0, as
    /// libgit2 leaves a signature without a timezone). Read and write its fields
    /// through [`as_ref`](Self::as_ref) / [`as_mut`](Self::as_mut).
    ///
    /// Wraps: git_time
    #[derive(Clone, Copy)]
    Time,
    TimeRef,
    TimeMut,
    ffi::git_time
);

// SAFETY: all zero is the epoch at offset 0 with sign byte 0, the value
// `git_signature_from_buffer` leaves for a signature without a timezone: every field is
// a plain integer, which the getters read and libgit2 formats, and there is no teardown.
unsafe impl CZeroable for Time {}

// SAFETY: a `Time` is 16 plain bytes: no pointer, no interior mutability, no thread
// affinity, nothing to release, so moving one to another thread moves bytes.
unsafe impl Send for Time {}
// SAFETY: `TimeMut` is an exclusive borrow of such a value, as `&mut Time`.
unsafe impl Send for TimeMut<'_> {}

// SAFETY: shared access only reads the fields (every write goes through `TimeMut`,
// which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for Time {}
// SAFETY: as above; `TimeRef` is a shared borrow, as `&Time`.
unsafe impl Send for TimeRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for TimeRef<'_> {}
// SAFETY: `&TimeMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for TimeMut<'_> {}

impl Time {
    /// The time `seconds` after the epoch in the zone `offset_minutes` east of UTC,
    /// signed as `git_signature_new` signs it: `'-'` for a negative offset, `'+'`
    /// otherwise. Any value is accepted, as libgit2 accepts it; see the [module
    /// docs](self) for what libgit2 then writes of a time outside `u32` seconds or at
    /// `i32::MIN` minutes.
    ///
    /// Handwritten
    #[must_use]
    pub fn new(seconds: i64, offset_minutes: i32) -> Time {
        let mut time = Time::zeroed();
        let mut handle = time.as_mut();
        handle.set_seconds(seconds);
        handle.set_offset_minutes(offset_minutes);
        handle.set_sign(if offset_minutes < 0 { b'-' } else { b'+' });
        time
    }
}

/// The `git_time` value as it is, for code that already holds one by value.
impl From<ffi::git_time> for Time {
    fn from(raw: ffi::git_time) -> Time {
        Time(raw, PhantomData)
    }
}

/// The `git_time` value, for code that passes one by value.
impl From<Time> for ffi::git_time {
    fn from(time: Time) -> ffi::git_time {
        time.0
    }
}

impl TimeRef<'_> {
    /// Seconds since the epoch, 1970-01-01T00:00:00Z; negative before it.
    ///
    /// Field: `git_time.time`
    #[must_use]
    pub fn seconds(&self) -> i64 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let time_ptr = unsafe { &raw const (*self.as_ptr()).time };
        // SAFETY: a read through the shared handle's pointer to a live `git_time`; any
        // value is a valid `i64`.
        unsafe { time_ptr.read() }
    }

    /// The timezone's offset from UTC in minutes, negative west of it. libgit2 parses
    /// at most 14 hours and 59 minutes either way, but stores whatever it is given.
    ///
    /// Field: `git_time.offset`
    #[must_use]
    pub fn offset_minutes(&self) -> i32 {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let offset_ptr = unsafe { &raw const (*self.as_ptr()).offset };
        // SAFETY: a read through the shared handle's pointer to a live `git_time`; any
        // value is a valid `i32`.
        unsafe { offset_ptr.read() }
    }

    /// The sign character the offset was written with, as a byte: `b'+'` or `b'-'`
    /// for every time libgit2 builds, 0 where a parsed signature had no valid
    /// timezone, or whatever character a malformed one started with. `b'-'` with an
    /// offset of 0 is git's questionable `-0000`.
    ///
    /// Field: `git_time.sign`
    #[must_use]
    pub fn sign(&self) -> u8 {
        // SAFETY: the shared handle's pointer addresses a live `git_time`; the projection forms no
        // reference to it.
        let sign_ptr = unsafe { &raw const (*self.as_ptr()).sign };
        // SAFETY: reads the initialised field; any `char` is a valid byte.
        unsafe { sign_ptr.read() }.cast_unsigned()
    }

    /// A copy of the time, as a value of its own.
    #[must_use]
    pub fn to_time(&self) -> Time {
        Time::from(ffi::git_time {
            time: self.seconds(),
            offset: self.offset_minutes(),
            sign: self.sign().cast_signed(),
        })
    }
}

impl TimeMut<'_> {
    /// Replaces the seconds since the epoch.
    ///
    /// Field: `git_time.time`
    pub fn set_seconds(&mut self, seconds: i64) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let time_ptr = unsafe { &raw mut (*self.as_mut_ptr()).time };
        // SAFETY: a write through the exclusive handle's pointer to a live `git_time`;
        // the field is a plain integer.
        unsafe { time_ptr.write(seconds) }
    }

    /// Replaces the offset from UTC in minutes. The sign character is left as it is.
    /// Any value is accepted; `i32::MIN` is one libgit2 cannot write (see the [module
    /// docs](self)).
    ///
    /// Field: `git_time.offset`
    pub fn set_offset_minutes(&mut self, offset_minutes: i32) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let offset_ptr = unsafe { &raw mut (*self.as_mut_ptr()).offset };
        // SAFETY: a write through the exclusive handle's pointer to a live `git_time`;
        // the field is a plain integer.
        unsafe { offset_ptr.write(offset_minutes) }
    }

    /// Replaces the sign character. Any byte is accepted, as libgit2 accepts it.
    ///
    /// Field: `git_time.sign`
    pub fn set_sign(&mut self, sign: u8) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let sign_ptr = unsafe { &raw mut (*self.as_mut_ptr()).sign };
        // SAFETY: a write through the exclusive handle's pointer to a live `git_time`;
        // the field is a plain `char`, which every byte fits.
        unsafe { sign_ptr.write(sign.cast_signed()) }
    }
}

impl fmt::Debug for TimeRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Time")
            .field("seconds", &self.seconds())
            .field("offset_minutes", &self.offset_minutes())
            .field("sign", &char::from(self.sign()))
            .finish()
    }
}

impl fmt::Debug for TimeMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl fmt::Debug for Time {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<TimeRef<'b>> for TimeRef<'_> {
    /// Field by field, as `git_signature__equal` compares times; padding is ignored.
    fn eq(&self, other: &TimeRef<'b>) -> bool {
        self.seconds() == other.seconds()
            && self.offset_minutes() == other.offset_minutes()
            && self.sign() == other.sign()
    }
}

impl Eq for TimeRef<'_> {}

impl Hash for TimeRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.seconds().hash(state);
        self.offset_minutes().hash(state);
        self.sign().hash(state);
    }
}

impl PartialEq for Time {
    fn eq(&self, other: &Self) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for Time {}

impl Hash for Time {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_ref().hash(state);
    }
}

// A `Time` is a `git_time`, so C reads and writes a value Rust holds in place.
const _: () = assert!(
    size_of::<Time>() == size_of::<ffi::git_time>()
        && align_of::<Time>() == align_of::<ffi::git_time>()
);

define_ctype!(
    /// An action signature (`git_signature`): who did something, as a name and an
    /// email, and when, as a [`Time`]; what commits, tags, notes and reflog entries
    /// record of their authors.
    ///
    /// Only ever behind a pointer (see the [module docs](self#signatures)): owned as a
    /// [`SignatureBox`], which [`new`](Self::new), [`now`](Self::now) and
    /// [`parse`](Self::parse) build; borrowed as a [`SignatureRef`] from whatever holds
    /// it, or from a [`SignatureView`] over borrowed strings. Only an owned signature
    /// has a [`SignatureMut`], whose setters free the strings they replace.
    ///
    /// There is no bare value to hold: one would leak its strings when dropped.
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::core::types::Signature;
    ///
    /// let _ = <Signature as ffibox::CZeroable>::zeroed();
    /// ```
    ///
    /// Wraps: git_signature
    Signature,
    SignatureRef,
    SignatureMut,
    ffi::git_signature
);

// SAFETY: an owned signature is a struct and its two strings, all allocations of
// libgit2's allocator that the struct owns exclusively; `git__free` releases them on any
// thread, and no libgit2 routine keeps thread-local state about them, so moving the owner
// moves sole ownership of them.
unsafe impl Send for Signature {}
// SAFETY: `SignatureMut` is an exclusive borrow of such a signature, as `&mut Signature`.
unsafe impl Send for SignatureMut<'_> {}

// SAFETY: shared access only reads the struct and the strings it addresses: every write
// goes through `SignatureMut`, which excludes other handles, and no libgit2 routine
// writes through a `const git_signature *`.
unsafe impl Sync for Signature {}
// SAFETY: as above; `SignatureRef` is a shared borrow, as `&Signature`. The signature it
// reads may also be a `SignatureView`'s, over strings borrowed as `&CStr`, which is `Sync`.
unsafe impl Send for SignatureRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for SignatureRef<'_> {}
// SAFETY: `&SignatureMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for SignatureMut<'_> {}

/// Releases a signature through `git_signature_free`, which hands its name and email,
/// then the struct, to `git__free`; copies one through `git_signature_dup`.
///
/// A copy needs both strings: `git_signature_dup` hands each to `strdup`, which a NULL
/// one (left by an allocation failure while libgit2 parsed the signature) would crash.
/// The policy refuses such a copy instead, so [`try_clone`](CBox::try_clone) gives
/// `None` and `Clone` aborts, as it does when the copy runs out of memory;
/// [`SignatureRef::to_signature`] reports it as an error.
///
/// Wraps: `git_signature_free`
/// Wraps: `git_signature_dup`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct SignatureFree;

/// An owned [`Signature`], freed by `git_signature_free` on drop; `Clone` copies it
/// through `git_signature_dup`.
pub type SignatureBox = CBox<Signature, SignatureFree>;

impl_cdrop!(SignatureFree, Signature, ffi::git_signature_free);
impl_cdupclone!(SignatureFree, Signature, signature_dup);

/// `git_signature_dup` in `impl_cdupclone!`'s shape: a fresh copy, or NULL when the
/// copy fails, libgit2 cannot be initialised, or `sig` lacks its name or its email.
///
/// # Safety
///
/// `sig` must address a live `git_signature` whose `name` and `email` are each NULL or
/// a NUL-terminated string, none of them written during the call.
unsafe fn signature_dup(sig: *mut ffi::git_signature) -> *mut ffi::git_signature {
    // SAFETY: per this function's contract `sig` addresses a live `git_signature`; the projection
    // forms no reference to it.
    let name_ptr = unsafe { &raw const (*sig).name };
    // SAFETY: as above.
    let email_ptr = unsafe { &raw const (*sig).email };
    // SAFETY: reads the initialised field.
    let name = unsafe { name_ptr.read() };
    // SAFETY: as above.
    let email = unsafe { email_ptr.read() };
    if name.is_null() || email.is_null() || !runtime_pinned() {
        return ptr::null_mut();
    }
    let mut copy = ptr::null_mut();
    // SAFETY: `copy` is a writable slot; `sig` is a live signature with two
    // NUL-terminated strings, only read. libgit2 writes the slot only on success, with a
    // fresh signature the caller owns. The runtime is pinned, so a failure is reported on
    // live thread-local state.
    let rc = unsafe { ffi::git_signature_dup(&raw mut copy, sig) };
    if rc < 0 { ptr::null_mut() } else { copy }
}

/// The signature a constructor returning `rc` handed out, or libgit2's error.
///
/// A signature without its name or its email is refused, with the out-of-memory error
/// that lost it: `git_signature_from_buffer` reports success when it could not allocate
/// one of them. That error is set again here, since the parser may have replaced it
/// before succeeding: `git__strntol32` reports a malformed timezone (`+abc`) as a
/// conversion error, which the parser then ignores.
fn constructed(rc: c_int, signature: Option<SignatureBox>) -> Result<SignatureBox, ErrorBuf> {
    let signature = signature.filter(|_| rc >= 0).ok_or_else(last)?;
    let complete = {
        let r = signature.as_ref();
        r.name().is_some() && r.email().is_some()
    };
    if complete {
        Ok(signature)
    } else {
        set_oom();
        Err(last())
    }
}

impl Signature {
    /// A signature of `name` and `email` at `seconds` since the epoch, in the zone
    /// `offset_minutes` east of UTC, signed as [`Time::new`] signs it.
    ///
    /// Wraps: `git_signature_new`
    ///
    /// # Errors
    ///
    /// libgit2 fails, with its error, when either string contains `<` or `>`; otherwise
    /// it trims both of the bytes git drops at their ends (every byte up to and including
    /// the space, and `,:;"\'`), and fails when either is then empty. Any time is
    /// accepted; see the [module docs](self) for what libgit2 then writes of a time
    /// outside `u32` seconds or at `i32::MIN` minutes.
    ///
    /// When libgit2 runs out of memory copying a string, it leaks the half-built
    /// signature before reporting the error.
    pub fn new(
        name: &CStr,
        email: &CStr,
        seconds: i64,
        offset_minutes: i32,
    ) -> Result<SignatureBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `name` and `email` are non-NULL
        // NUL-terminated strings, as libgit2 asserts, read for the call and copied, never
        // retained. The runtime is pinned, so a failure is reported on live thread-local
        // state.
        let rc = unsafe {
            ffi::git_signature_new(
                &raw mut out,
                name.as_ptr(),
                email.as_ptr(),
                seconds,
                offset_minutes,
            )
        };
        // SAFETY: `git_signature_new` leaves `out` NULL on failure (adopted as `None`), or
        // writes a fresh signature from libgit2's allocator that the caller owns alone,
        // released by `git_signature_free` as `SignatureFree` does.
        constructed(rc, unsafe { SignatureBox::from_c(out) })
    }

    /// As [`new`](Self::new), at the current time in the local timezone: the system
    /// clock's seconds, and the offset C's `mktime` derives for them.
    ///
    /// Concurrent calls are sound: glibc serialises its timezone state under a lock of
    /// its own. `ThreadSanitizer` cannot see that lock, and reports them as a race on
    /// glibc's copy of `TZ`.
    ///
    /// Wraps: `git_signature_now`
    ///
    /// # Errors
    ///
    /// Fails as [`new`](Self::new) does, and with libgit2's error when the system clock or the
    /// local timezone cannot be read.
    pub fn now(name: &CStr, email: &CStr) -> Result<SignatureBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: as in `new`.
        let rc = unsafe { ffi::git_signature_now(&raw mut out, name.as_ptr(), email.as_ptr()) };
        // SAFETY: `git_signature_now` leaves `out` NULL on failure (adopted as `None`), or
        // writes a fresh signature from libgit2's allocator that the caller owns alone,
        // released by `git_signature_free` as `SignatureFree` does.
        constructed(rc, unsafe { SignatureBox::from_c(out) })
    }

    /// Parses the signature line `buf`, as git writes one after `author `: a name, an
    /// email between `<` and `>`, then optionally the seconds and the zone, as in
    /// `A U Thor <author@example.com> 1700000000 +0130`.
    ///
    /// The name and email are trimmed as [`new`](Self::new) trims them, but may contain angle
    /// brackets or be empty. A missing time leaves the epoch, and the zone is kept as libgit2 reads
    /// it, a malformed one included (see [`Time`]).
    ///
    /// Wraps: `git_signature_from_buffer`
    ///
    /// # Errors
    ///
    /// Fails, with libgit2's error, without an email in angle brackets or with seconds that do not
    /// parse.
    ///
    /// libgit2 reports success when it runs out of memory copying the name or the
    /// email, leaving it NULL; that signature is refused here, with the out-of-memory
    /// error, so every signature this builds has both.
    pub fn parse(buf: &CStr) -> Result<SignatureBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot and `buf` a non-NULL NUL-terminated string, as
        // libgit2 asserts, read up to its NUL for the call only. The runtime is pinned.
        let rc = unsafe { ffi::git_signature_from_buffer(&raw mut out, buf.as_ptr()) };
        // SAFETY: `git_signature_from_buffer` leaves `out` NULL on failure (adopted as `None`), or
        // writes a fresh signature from libgit2's allocator that the caller owns alone,
        // released by `git_signature_free` as `SignatureFree` does.
        constructed(rc, unsafe { SignatureBox::from_c(out) })
    }
}

impl<'a> SignatureRef<'a> {
    /// The name, borrowed in place for as long as the signature is; `None` only where
    /// libgit2 ran out of memory copying it while it parsed the signature (a commit's,
    /// say); every signature this crate builds has one.
    ///
    /// Field: `git_signature.name`
    #[must_use]
    pub fn name(&self) -> Option<&'a CStr> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let name_ptr = unsafe { &raw const (*self.as_ptr()).name };
        // SAFETY: a read through the shared handle's pointer to a live `git_signature`.
        let name = unsafe { name_ptr.read() };
        // SAFETY: per the handles' invariant a non-NULL name is a NUL-terminated string
        // that outlives the handle, i.e. `'a`; nothing writes or frees it while a shared
        // handle lives, since that needs the exclusive one.
        (!name.is_null()).then(|| unsafe { CStr::from_ptr(name) })
    }

    /// The email, borrowed in place for as long as the signature is; `None` only where
    /// libgit2 ran out of memory copying it, as for [`name`](Self::name).
    ///
    /// Field: `git_signature.email`
    #[must_use]
    pub fn email(&self) -> Option<&'a CStr> {
        // SAFETY: `self.as_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let email_ptr = unsafe { &raw const (*self.as_ptr()).email };
        // SAFETY: a read through the shared handle's pointer to a live `git_signature`.
        let email = unsafe { email_ptr.read() };
        // SAFETY: as in `name`.
        (!email.is_null()).then(|| unsafe { CStr::from_ptr(email) })
    }

    /// When the action happened, borrowed in place for as long as the signature is.
    ///
    /// Field: `git_signature.when`
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a field's address is not NULL"
    )]
    pub fn when(&self) -> TimeRef<'a> {
        // SAFETY: a projection, without a read, of a live `git_signature`'s field.
        let when = unsafe { addr_of!((*self.as_ptr()).when) };
        // SAFETY: `when` is the embedded `git_time` of a signature that lives for `'a`,
        // initialised as every field is, and only read through the shared handle.
        unsafe { TimeRef::from_ptr(when.cast_mut()) }.expect("a field's address is not NULL")
    }

    /// A copy of the signature in libgit2's allocator, its strings copied, as
    /// `git_signature_dup` makes one. [`SignatureView`]s and the signatures libgit2's
    /// objects hold become owned this way; an owner also copies with `Clone`.
    ///
    /// Wraps: `git_signature_dup`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's error when it runs out of memory, or cannot be initialised;
    /// and, with a `GIT_ERROR_INVALID` error set on the thread, for a signature without
    /// its name or its email, which `git_signature_dup` would crash on.
    pub fn to_signature(&self) -> Result<SignatureBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        if self.name().is_none() || self.email().is_none() {
            set(
                ErrorClass::Invalid,
                c"failed to copy signature - missing name or email",
            );
            return Err(last());
        }
        // SAFETY: the handle addresses a live signature whose name and email are
        // NUL-terminated strings (checked non-NULL above), which nothing writes while
        // the shared handle lives; `git_signature_dup` only reads it.
        let copy = unsafe { signature_dup(self.as_ptr().cast_mut()) };
        // SAFETY: NULL, or a fresh signature from `git_signature_dup` the caller owns,
        // released by `git_signature_free` as `SignatureFree` does.
        unsafe { SignatureBox::from_c(copy) }.ok_or_else(last)
    }
}

impl SignatureMut<'_> {
    /// Replaces the name, releasing the one held before.
    ///
    /// Any string is accepted, as a C caller writing the field may: libgit2 writes a
    /// name into commits and tags as it is, so one holding `<`, `>` or a newline is
    /// written into an object git cannot parse back. [`Signature::new`] refuses those
    /// brackets.
    ///
    /// Field: `git_signature.name`
    pub fn set_name(&mut self, name: OdbDataStr) {
        drop(self.replace_name(name));
    }

    /// Replaces the name, handing back the one held before (`None` where libgit2 had
    /// lost it, as [`SignatureRef::name`] reads it).
    ///
    /// Field: `git_signature.name`
    pub fn replace_name(&mut self, name: OdbDataStr) -> Option<OdbDataStr> {
        // SAFETY: a projection, without a read, of a live `git_signature`'s field
        // through the exclusive handle's pointer.
        let field = unsafe { addr_of_mut!((*self.as_mut_ptr()).name) };
        // SAFETY: `field` is the live, exclusively borrowed `name` slot.
        unsafe { replace_string(field, name) }
    }

    /// Replaces the email, releasing the one held before. Any string is accepted, as
    /// for [`set_name`](Self::set_name); an email holding `>` or a newline breaks the
    /// objects libgit2 writes it into.
    ///
    /// Field: `git_signature.email`
    pub fn set_email(&mut self, email: OdbDataStr) {
        drop(self.replace_email(email));
    }

    /// Replaces the email, handing back the one held before (`None` where libgit2 had
    /// lost it).
    ///
    /// Field: `git_signature.email`
    pub fn replace_email(&mut self, email: OdbDataStr) -> Option<OdbDataStr> {
        // SAFETY: as in `replace_name`.
        let field = unsafe { addr_of_mut!((*self.as_mut_ptr()).email) };
        // SAFETY: `field` is the live, exclusively borrowed `email` slot.
        unsafe { replace_string(field, email) }
    }

    /// When the action happened, for its setters, borrowed in place.
    ///
    /// Field: `git_signature.when`
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a field's address is not NULL"
    )]
    pub fn when_mut(&mut self) -> TimeMut<'_> {
        // SAFETY: a projection, without a read, of a live `git_signature`'s field
        // through the exclusive handle's pointer.
        let when = unsafe { addr_of_mut!((*self.as_mut_ptr()).when) };
        // SAFETY: `when` is the embedded `git_time` of a signature this handle borrows
        // exclusively for the result's lifetime, which the result reborrows.
        unsafe { TimeMut::from_ptr(when) }.expect("a field's address is not NULL")
    }

    /// Replaces when the action happened.
    ///
    /// Field: `git_signature.when`
    pub fn set_when(&mut self, when: Time) {
        // SAFETY: `self.as_mut_ptr()` addresses the live object this handle borrows; the projection
        // forms no reference to it.
        let when_ptr = unsafe { &raw mut (*self.as_mut_ptr()).when };
        // SAFETY: a write through the exclusive handle's pointer to a live
        // `git_signature`; `git_time` is plain data, with nothing to release.
        unsafe { when_ptr.write(when.into()) }
    }
}

/// Stores `new` in the string slot `field` and hands back what it held.
///
/// # Safety
///
/// `field` must be the `name` or `email` slot of a live signature behind a
/// [`SignatureMut`], borrowed exclusively for the call: per the handles' invariant it
/// holds NULL or a string from libgit2's allocator that the signature owns.
unsafe fn replace_string(field: *mut *mut c_char, new: OdbDataStr) -> Option<OdbDataStr> {
    // SAFETY: per this function's contract the slot is live and exclusively borrowed;
    // its old string's ownership leaves the signature as the new one's enters it.
    let old = unsafe { field.replace(new.into_raw()) };
    // SAFETY: `old` is NULL or a string from libgit2's allocator the signature owned and
    // no longer refers to, released by `git__free` as `OdbDataStrFree` does.
    unsafe { OdbDataStr::from_raw(old) }
}

impl fmt::Debug for SignatureRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Signature")
            .field("name", &self.name())
            .field("email", &self.email())
            .field("when", &self.when())
            .finish()
    }
}

impl fmt::Debug for SignatureMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'b> PartialEq<SignatureRef<'b>> for SignatureRef<'_> {
    /// Field by field, as `git_signature__equal` compares signatures: the strings by
    /// their bytes, the times as [`TimeRef`] compares them.
    fn eq(&self, other: &SignatureRef<'b>) -> bool {
        self.name() == other.name() && self.email() == other.email() && self.when() == other.when()
    }
}

impl Eq for SignatureRef<'_> {}

/// A signature (`git_signature`) over borrowed strings, for the `const git_signature *`
/// parameters libgit2 only reads: what C code builds on the stack as
/// `git_signature sig = { name, email, when }`.
///
/// The struct is Rust's own, and the strings stay the caller's: nothing is allocated from
/// libgit2, nothing reaches `git_signature_free`, and the view only hands out the shared
/// [`SignatureRef`], so no setter can free what it borrows.
/// [`SignatureRef::to_signature`] copies it into an owned signature.
///
/// Unlike [`Signature::new`], a view takes its strings as they are: libgit2 writes them
/// into commits and tags without checking them, so a name or email holding angle
/// brackets or a newline makes an object git cannot parse back.
///
/// There is no exclusive handle:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::core::types::{SignatureView, Time};
///
/// let mut view = SignatureView::new(c"A", c"a@b", Time::new(0, 0));
/// let _ = view.as_mut();
/// ```
///
/// Wraps: `git_signature`
/// Handwritten
pub struct SignatureView<'a> {
    /// `name` and `email` address the borrowed strings.
    signature: Signature,
    _strs: PhantomData<&'a CStr>,
}

// SAFETY: a view is a `git_signature` over two strings borrowed as `&'a CStr`, which is
// `Send` and `Sync`, and a plain `git_time`; nothing writes it after construction, and
// the strings are never freed through it.
unsafe impl Send for SignatureView<'_> {}
// SAFETY: as above: shared access only reads.
unsafe impl Sync for SignatureView<'_> {}

impl<'a> SignatureView<'a> {
    /// A view of `name` and `email`, at `when`.
    #[must_use]
    pub fn new(name: &'a CStr, email: &'a CStr, when: Time) -> Self {
        let raw = ffi::git_signature {
            name: name.as_ptr().cast_mut(),
            email: email.as_ptr().cast_mut(),
            when: when.into(),
        };
        Self {
            signature: Signature(raw, PhantomData),
            _strs: PhantomData,
        }
    }

    /// The shared handle, for reading the signature or passing it to C.
    #[must_use]
    pub fn as_ref(&self) -> SignatureRef<'_> {
        self.signature.as_ref()
    }
}

impl Clone for SignatureView<'_> {
    fn clone(&self) -> Self {
        Self {
            signature: Signature(self.signature.0, PhantomData),
            _strs: PhantomData,
        }
    }
}

impl fmt::Debug for SignatureView<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// A `Signature` is a `git_signature`, so C reads one in place, and owns one through a
// `SignatureBox`; a view's `Signature` is held by value.
const _: () = assert!(
    size_of::<Signature>() == size_of::<ffi::git_signature>()
        && align_of::<Signature>() == align_of::<ffi::git_signature>()
);

define_ctype!(
    /// A stream that data is written into piece by piece, then closed
    /// (`git_writestream`): the sink of libgit2's filters, and the writer
    /// `git_blob_create_from_stream` opens.
    ///
    /// Only ever behind a pointer, in the container its producer embedded it in (see
    /// the [module docs](self#write-streams)): owned as a [`WriteStreamBox`] or a
    /// [`RustWriteStream`], borrowed as a [`WriteStreamRef`] / [`WriteStreamMut`].
    /// There is no public constructor of a bare header, and no setter: replacing a
    /// member would detach it from the container its functions cast the header to.
    ///
    /// Besides `from_ptr`'s requirements, building a handle requires the pointer to be
    /// a header in place in its container, its three members set to that container's
    /// functions, and the stream to accept writes for the handle's lifetime: not
    /// closed, unless it is a [`RustWriteStream`]'s, whose members accept any order of
    /// calls. Lending a handle to C code that closes the stream (as
    /// `git_filter_list_stream_*` do with their target) is sound only for such a
    /// stream, or when the handle is not written through again.
    ///
    /// The handles are neither `Send` nor `Sync`: a C stream may hold a repository or
    /// a filter session that belongs to its thread.
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::core::types::WriteStreamBox;
    ///
    /// fn send<T: Send>(_: T) {}
    /// fn forward(stream: WriteStreamBox) {
    ///     send(stream);
    /// }
    /// ```
    ///
    /// Wraps: git_writestream
    WriteStream,
    WriteStreamRef,
    WriteStreamMut,
    ffi::git_writestream
);

impl WriteStream {
    /// Closes the stream, then frees it: its `close` member, then its `free` member.
    ///
    /// Closing ends a stream, so the owner is consumed and freed whether or not
    /// closing fails. A negative return is an error, which comes back with the thread's
    /// last libgit2 error; any other return is success, as libgit2's callers read it.
    ///
    /// Not every stream is meant to be closed by its owner: the stream
    /// `git_blob_create_from_stream` opens is committed instead, and closing it
    /// discards what was written (dropping it does the same).
    ///
    /// Field: `git_writestream.close`
    ///
    /// # Errors
    ///
    /// A negative return from `close` is an error, which comes back with the thread's last libgit2
    /// error; the stream is freed either way.
    pub fn close(mut stream: WriteStreamBox) -> Result<(), StreamError> {
        // SAFETY: `stream` is dropped right below, so the stream is freed and never
        // written or closed again.
        let closed = unsafe { stream.as_mut().close() };
        drop(stream);
        closed
    }
}

impl WriteStreamMut<'_> {
    /// Writes `data` into the stream: its `write` member, given the slice's pointer
    /// and length. The stream reads the bytes during the call only.
    ///
    /// # Errors
    ///
    /// A negative return is an error, which comes back with the thread's last libgit2
    /// error; any other return is success, as libgit2's callers read it.
    ///
    /// # Panics
    ///
    /// If the stream has no `write` member, which no producer leaves unset and the
    /// handle's invariant excludes.
    ///
    /// Field: `git_writestream.write`
    pub fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
        let stream = self.as_mut_ptr();
        // SAFETY: `stream` is the handle's pointer to the live object it borrows; the projection
        // forms no reference to it.
        let write_ptr = unsafe { &raw const (*stream).write };
        // SAFETY: a read through the exclusive handle's pointer to a live header.
        let write = unsafe { write_ptr.read() };
        let write = write.expect("a git_writestream always has a write member");
        // SAFETY: per the handle's invariant, `write` is the member the stream's
        // producer set for this header, in place in its container, and the stream
        // accepts writes; it reads `data.len()` bytes at `data` during the call only.
        // The handle is exclusive, so no other call on the stream runs meanwhile.
        let rc = unsafe { write(stream, data.as_ptr().cast(), data.len()) };
        check(rc)
    }

    /// Closes the stream: its `close` member. Results read as [`write`](Self::write)'s.
    ///
    /// # Panics
    ///
    /// If the stream has no `close` member, which the handle's invariant excludes.
    ///
    /// # Safety
    ///
    /// Closing ends the stream: afterwards it must not be written or closed again
    /// through any handle, only freed, unless it is a [`RustWriteStream`]'s.
    ///
    /// Field: `git_writestream.close`
    pub(crate) unsafe fn close(&mut self) -> Result<(), StreamError> {
        let stream = self.as_mut_ptr();
        // SAFETY: `stream` is the handle's pointer to the live object it borrows; the projection
        // forms no reference to it.
        let close_ptr = unsafe { &raw const (*stream).close };
        // SAFETY: a read through the exclusive handle's pointer to a live header.
        let close = unsafe { close_ptr.read() };
        let close = close.expect("a git_writestream always has a close member");
        // SAFETY: as in `write`, for the `close` member; the caller ends the stream.
        let rc = unsafe { close(stream) };
        check(rc)
    }
}

/// A stream member's return as a result: negative is an error, described by the
/// thread's last libgit2 error, which is copied before any other libgit2 call.
fn check(rc: c_int) -> Result<(), StreamError> {
    if rc < 0 {
        Err(StreamError::from_libgit2(rc))
    } else {
        Ok(())
    }
}

/// `git_writestream.free` in `impl_cdrop!`'s shape: the stream's own destructor.
///
/// A stream without one is left alone: no producer leaves the member unset.
///
/// # Safety
///
/// `stream` is a live header in its container, with its members set, which the caller
/// owns and does not use again.
unsafe fn stream_free(stream: *mut ffi::git_writestream) {
    // SAFETY: per this function's contract the caller owns the live header; the projection forms no
    // reference to it.
    let free_ptr = unsafe { &raw const (*stream).free };
    // SAFETY: reads the initialised member.
    let free = unsafe { free_ptr.read() };
    if let Some(free) = free {
        // SAFETY: `free` is the member the stream's producer set for this header; it
        // releases the container, which the caller gives up.
        unsafe { free(stream) }
    }
}

/// Releases a stream through its own `free` member, which frees the container the
/// header is embedded in: a blob writer's file buffer, hint path and allocation; a
/// [`RustWriteStream`]'s box. Stateless: the routine is recovered from the object.
///
/// Field: `git_writestream.free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteStreamFree;

impl_cdrop!(WriteStreamFree, WriteStream, stream_free);

/// The sole owner of a stream, C's or Rust's, released through its `free` member.
///
/// [`RustWriteStream::into_box`] builds one safely; a stream libgit2 opens is adopted
/// by the wrapper of the routine that opens it.
pub type WriteStreamBox = CBox<WriteStream, WriteStreamFree>;

/// What a stream implemented in Rust does when its `write` and `close` members are
/// called, by libgit2 or through a handle; [`RustWriteStream`] adapts it.
///
/// libgit2 writes any number of times and then closes once, but a stream lent to other
/// code may see any order, so an implementation must not rely on one. An error is
/// returned to the caller as its code and becomes the thread's last libgit2 error. A
/// panic aborts the process, since it cannot unwind into C.
///
/// Handwritten
pub trait StreamWriter {
    /// Takes the next piece of the stream's data.
    ///
    /// # Errors
    ///
    /// An error ends the producer's write: libgit2 reports the failure of the operation that
    /// streams into it.
    fn write(&mut self, data: &[u8]) -> Result<(), StreamError>;

    /// Ends the stream's data. The default accepts.
    ///
    /// # Errors
    ///
    /// An error fails the operation that streams into it, after the data was written.
    fn close(&mut self) -> Result<(), StreamError> {
        Ok(())
    }
}

/// Collects the data written.
impl StreamWriter for Vec<u8> {
    fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
        self.extend_from_slice(data);
        Ok(())
    }
}

/// Forwards to the borrowed writer, which the caller keeps.
impl<T: StreamWriter + ?Sized> StreamWriter for &mut T {
    fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
        (**self).write(data)
    }

    fn close(&mut self) -> Result<(), StreamError> {
        (**self).close()
    }
}

/// A stream's header and the writer its members run, boxed together: the header first,
/// so a pointer to it is a pointer to the box.
#[repr(C)]
struct Repr<W> {
    stream: ffi::git_writestream,
    writer: W,
}

/// A stream implemented in Rust: a `git_writestream` whose members run a
/// [`StreamWriter`], owned by Rust.
///
/// It lends [`WriteStreamRef`] / [`WriteStreamMut`] handles, as any owner does, and the
/// writer itself, through [`writer`](Self::writer) / [`writer_mut`](Self::writer_mut)
/// or [`into_inner`](Self::into_inner) once the stream is done. Its members accept any
/// order of calls, so it may be lent to C code that closes it and written again
/// afterwards. [`into_box`](Self::into_box) erases the writer's type into a
/// [`WriteStreamBox`], for code that takes any stream (a filter's `stream` callback
/// hands one to libgit2, which frees it); dropping either drops the writer.
///
/// ```
/// use libgit2::libgit2::core::types::RustWriteStream;
///
/// let mut stream = RustWriteStream::new(Vec::new());
/// stream.as_mut().write(b"hello, ").unwrap();
/// stream.as_mut().write(b"world").unwrap();
/// assert_eq!(stream.into_inner(), b"hello, world");
/// ```
///
/// A handle borrows the stream, and an erased one cannot borrow anything:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::core::types::RustWriteStream;
///
/// let mut out = Vec::new();
/// let stream = RustWriteStream::new(&mut out);
/// let _boxed = stream.into_box();
/// ```
///
/// It crosses threads only with a writer that does:
///
/// ```compile_fail,E0277
/// use std::rc::Rc;
///
/// use libgit2::libgit2::core::types::{RustWriteStream, StreamError, StreamWriter};
///
/// struct Shared(Rc<()>);
///
/// impl StreamWriter for Shared {
///     fn write(&mut self, _: &[u8]) -> Result<(), StreamError> {
///         Ok(())
///     }
/// }
///
/// let stream = RustWriteStream::new(Shared(Rc::new(())));
/// std::thread::spawn(move || drop(stream));
/// ```
///
/// Wraps: `git_writestream`
/// Handwritten
pub struct RustWriteStream<W: StreamWriter> {
    raw: NonNull<Repr<W>>,
    _owns: PhantomData<Box<Repr<W>>>,
}

// SAFETY: a `RustWriteStream` uniquely owns its box, as `Box<Repr<W>>` does: the header
// holds three function pointers fixed at construction, nothing else points into the
// box while the owner holds it, and no member call outlives the borrow that made it.
// Moving the owner to another thread moves `W` and three code addresses.
unsafe impl<W: StreamWriter + Send> Send for RustWriteStream<W> {}
// SAFETY: `&RustWriteStream` reaches `&W` ([`RustWriteStream::writer`]) and a shared
// handle, which neither calls a member nor writes the header; every call and write
// needs `&mut`.
unsafe impl<W: StreamWriter + Sync> Sync for RustWriteStream<W> {}

impl<W: StreamWriter> RustWriteStream<W> {
    /// A stream whose members run `writer`.
    pub fn new(writer: W) -> Self {
        let repr = Box::new(Repr {
            stream: ffi::git_writestream {
                write: Some(write_with::<W>),
                close: Some(close_with::<W>),
                free: Some(free_with::<W>),
            },
            writer,
        });
        Self {
            raw: NonNull::from(Box::leak(repr)),
            _owns: PhantomData,
        }
    }

    /// The stream's header.
    fn header(&self) -> *mut ffi::git_writestream {
        // The header is `Repr`'s first field, at offset 0 (`#[repr(C)]`).
        self.raw.as_ptr().cast()
    }

    /// Shared handle to the stream.
    #[must_use]
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a box is never NULL"
    )]
    pub fn as_ref(&self) -> WriteStreamRef<'_> {
        // SAFETY: the header of the live box this owner holds, in place in its `Repr`
        // with every member set to `Repr<W>`'s trampolines; shared for `&self`.
        unsafe { WriteStreamRef::from_ptr(self.header()) }.expect("a box is never NULL")
    }

    /// Exclusive handle to the stream, to write into it or lend it to code that does.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a box is never NULL"
    )]
    pub fn as_mut(&mut self) -> WriteStreamMut<'_> {
        // SAFETY: as in `as_ref`, exclusive for `&mut self`; the trampolines accept any
        // order of calls.
        unsafe { WriteStreamMut::from_ptr(self.header()) }.expect("a box is never NULL")
    }

    /// The writer.
    #[must_use]
    pub fn writer(&self) -> &W {
        // SAFETY: the writer of the live box this owner holds; no member call runs while `&self`
        // lives, since each needs a handle borrowed from `&mut self`.
        unsafe { &(*self.raw.as_ptr()).writer }
    }

    /// The writer, exclusively.
    pub fn writer_mut(&mut self) -> &mut W {
        // SAFETY: as in `writer`, exclusive for `&mut self`.
        unsafe { &mut (*self.raw.as_ptr()).writer }
    }

    /// The writer, the stream's header released.
    #[must_use]
    pub fn into_inner(self) -> W {
        let this = ManuallyDrop::new(self);
        // SAFETY: the box this owner held, from `Box::leak` in `new`; `this` is never
        // dropped, so the box is reclaimed once.
        let repr = unsafe { Box::from_raw(this.raw.as_ptr()) };
        repr.writer
    }

    /// The stream as an owner of any stream: it is freed through its `free` member,
    /// which drops the writer and the box.
    ///
    /// `WriteStreamBox` carries no lifetime, so the writer must borrow nothing.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: a box is never NULL"
    )]
    pub fn into_box(self) -> WriteStreamBox
    where
        W: 'static,
    {
        let this = ManuallyDrop::new(self);
        // SAFETY: the header of the live box this owner held, which it gives up (`this`
        // is never dropped); its `free` member, `free_with::<W>`, reclaims exactly that
        // box, and `W: 'static` outlives any owner.
        unsafe { WriteStreamBox::from_c(this.header()) }.expect("a box is never NULL")
    }
}

impl<W: StreamWriter> Drop for RustWriteStream<W> {
    fn drop(&mut self) {
        // SAFETY: the box this owner holds, from `Box::leak` in `new`, reclaimed once.
        drop(unsafe { Box::from_raw(self.raw.as_ptr()) });
    }
}

impl<W: StreamWriter + fmt::Debug> fmt::Debug for RustWriteStream<W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RustWriteStream")
            .field("writer", self.writer())
            .finish()
    }
}

/// The writer of the box `stream` heads.
///
/// # Safety
///
/// `stream` is the header of a live `Repr<W>`, which no other reference reaches for
/// `'a`.
unsafe fn writer_of<'a, W>(stream: *mut ffi::git_writestream) -> &'a mut W {
    // SAFETY: per this function's contract; the header is at offset 0 of its `Repr`.
    unsafe { &mut (*stream.cast::<Repr<W>>()).writer }
}

/// A writer's result as the member's return: 0, or the error's code once it is the
/// thread's last libgit2 error.
fn returned(result: Result<(), StreamError>) -> c_int {
    match result {
        Ok(()) => 0,
        Err(error) => error.raise(),
    }
}

/// `git_writestream.write` over [`StreamWriter::write`].
///
/// A NULL `buffer` is empty data when `len` is 0, and refused otherwise.
///
/// # Safety
///
/// `stream` is the header of a live `Repr<W>`, which nothing else reaches during the
/// call; `buffer` is NULL or valid for reads of `len` bytes, unchanged during the call.
unsafe extern "C" fn write_with<W: StreamWriter>(
    stream: *mut ffi::git_writestream,
    buffer: *const c_char,
    len: usize,
) -> c_int {
    let data: &[u8] = match (buffer.is_null(), len) {
        (_, 0) => &[],
        (true, _) => {
            let error = StreamError::new(ErrorClass::Invalid, c"NULL buffer written to stream");
            return error.raise();
        }
        // SAFETY: per this function's contract, `buffer` is valid for `len` bytes.
        (false, _) => unsafe { core::slice::from_raw_parts(buffer.cast(), len) },
    };
    // SAFETY: per this function's contract.
    let writer = unsafe { writer_of::<W>(stream) };
    returned(writer.write(data))
}

/// `git_writestream.close` over [`StreamWriter::close`].
///
/// # Safety
///
/// `stream` is the header of a live `Repr<W>`, which nothing else reaches during the
/// call.
unsafe extern "C" fn close_with<W: StreamWriter>(stream: *mut ffi::git_writestream) -> c_int {
    // SAFETY: per this function's contract.
    let writer = unsafe { writer_of::<W>(stream) };
    returned(writer.close())
}

/// `git_writestream.free`: drops the `Repr<W>` box, writer and all.
///
/// # Safety
///
/// `stream` is the header of a live `Repr<W>` box, which the caller owns and does not
/// use again.
unsafe extern "C" fn free_with<W: StreamWriter>(stream: *mut ffi::git_writestream) {
    // SAFETY: per this function's contract, the box from `RustWriteStream::new`, given
    // up by its owner.
    drop(unsafe { Box::from_raw(stream.cast::<Repr<W>>()) });
}

/// A stream's failure: the code its member returned, and the error it left as the
/// thread's last libgit2 error, copied out; or the failure a [`StreamWriter`] reports.
///
/// A writer's error reaches its caller the way C's do: its member returns the code,
/// and the class and message become the thread's last error.
///
/// Handwritten
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct StreamError {
    code: c_int,
    class: c_int,
    message: CString,
}

impl StreamError {
    /// A failure in `class` described by `message`, returned as `GIT_ERROR`.
    #[must_use]
    pub fn new(class: ErrorClass, message: &CStr) -> Self {
        Self {
            code: ErrorCode::Generic.raw(),
            class: class.raw(),
            message: message.to_owned(),
        }
    }

    /// The same failure, returned as `code` (`GIT_EUSER`, say). [`ErrorCode::Ok`]
    /// would read as success, so the code then stays `GIT_ERROR`.
    #[must_use]
    pub fn with_code(mut self, code: ErrorCode) -> Self {
        if code != ErrorCode::Ok {
            self.code = code.raw();
        }
        self
    }

    /// The returned code; `None` for one the header does not name, which a stream may
    /// return ([`raw_code`](Self::raw_code) has it).
    #[must_use]
    pub fn code(&self) -> Option<ErrorCode> {
        ErrorCode::from_raw(self.code)
    }

    /// The returned code, always negative.
    #[must_use]
    pub fn raw_code(&self) -> c_int {
        self.code
    }

    /// The error's class; `None` for a class the header does not name
    /// ([`raw_class`](Self::raw_class) has it).
    #[must_use]
    pub fn class(&self) -> Option<ErrorClass> {
        ErrorClass::from_raw(self.class)
    }

    /// The error's class as libgit2 stores it.
    #[must_use]
    pub fn raw_class(&self) -> c_int {
        self.class
    }

    /// The error message.
    #[must_use]
    pub fn message(&self) -> &CStr {
        &self.message
    }

    /// The failure a member reported by returning `code`, with the thread's last error.
    fn from_libgit2(code: c_int) -> Self {
        let error = last();
        Self {
            code,
            class: error.raw_class(),
            message: error.message().to_owned(),
        }
    }

    /// Makes this the thread's last libgit2 error (`git_error_set_str`), for a member to
    /// return its code. When libgit2 cannot be initialised the error is not stored, and
    /// when the thread's error buffer is out of memory libgit2 keeps the previous one;
    /// the code is returned either way.
    pub(crate) fn raise(&self) -> c_int {
        if runtime_pinned() {
            // SAFETY: `message` is a live C string, which libgit2 copies; any class is
            // accepted. The runtime is pinned, so the thread state is looked up under
            // libgit2's live TLS key.
            let _ = unsafe { ffi::git_error_set_str(self.class, self.message.as_ptr()) };
        }
        self.code
    }
}

/// A failure reported by another libgit2 wrapper, returned as `GIT_ERROR`.
impl From<ErrorBuf> for StreamError {
    fn from(error: ErrorBuf) -> Self {
        Self {
            code: ErrorCode::Generic.raw(),
            class: error.raw_class(),
            message: error.message().to_owned(),
        }
    }
}

impl fmt::Debug for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("StreamError");
        match self.code() {
            Some(code) => s.field("code", &code),
            None => s.field("code", &self.code),
        };
        match self.class() {
            Some(class) => s.field("class", &class),
            None => s.field("class", &self.class),
        };
        s.field("message", &self.message).finish()
    }
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message.to_string_lossy())
    }
}

impl std::error::Error for StreamError {}

#[cfg(test)]
mod unit_tests {
    use std::collections::HashSet;
    use std::collections::hash_map::DefaultHasher;
    use std::error::Error;

    use super::*;

    const OBJECT_TYPES: [(ffi::git_object_t, ObjectType); 4] = [
        (ffi::GIT_OBJECT_COMMIT, ObjectType::Commit),
        (ffi::GIT_OBJECT_TREE, ObjectType::Tree),
        (ffi::GIT_OBJECT_BLOB, ObjectType::Blob),
        (ffi::GIT_OBJECT_TAG, ObjectType::Tag),
    ];

    const REFERENCE_TYPES: [(ffi::git_reference_t, ReferenceType); 2] = [
        (ffi::GIT_REFERENCE_DIRECT, ReferenceType::Direct),
        (ffi::GIT_REFERENCE_SYMBOLIC, ReferenceType::Symbolic),
    ];

    /// Every value around the declared ones, and the extremes.
    fn object_probes() -> impl Iterator<Item = ffi::git_object_t> {
        (-300..=300).chain([
            ffi::git_object_t::MIN,
            ffi::git_object_t::MIN + 1,
            ffi::git_object_t::MAX - 1,
            ffi::git_object_t::MAX,
        ])
    }

    /// Every value around the declared ones, and the extremes; `u32::MAX` is the `-1`
    /// of `git_reference_type(NULL)`.
    fn reference_probes() -> impl Iterator<Item = ffi::git_reference_t> {
        (0..=300).chain([ffi::git_reference_t::MAX - 1, ffi::git_reference_t::MAX])
    }

    #[test]
    fn object_variants_carry_their_c_values() {
        for (raw, kind) in OBJECT_TYPES {
            assert_eq!(kind.raw(), raw);
            assert_eq!(ffi::git_object_t::from(kind), raw);
            assert_eq!(ObjectType::from_raw(raw), Some(kind));
            assert_eq!(ObjectType::try_from(raw), Ok(kind));
            assert_eq!(ObjectType::raw_or_any(Some(kind)), raw);
        }
        let distinct: HashSet<_> = OBJECT_TYPES.iter().map(|(_, k)| *k).collect();
        assert_eq!(distinct.len(), OBJECT_TYPES.len());
    }

    #[test]
    fn object_from_raw_refuses_every_other_value() {
        for raw in object_probes() {
            let named = OBJECT_TYPES
                .iter()
                .find(|(r, _)| *r == raw)
                .map(|(_, k)| *k);
            assert_eq!(ObjectType::from_raw(raw), named, "{raw}");
            match ObjectType::try_from(raw) {
                Ok(kind) => assert_eq!(Some(kind), named),
                Err(e) => {
                    assert_eq!(named, None);
                    assert_eq!(e.raw(), raw);
                }
            }
        }
        // The named non-types in particular.
        for raw in [ffi::GIT_OBJECT_ANY, ffi::GIT_OBJECT_INVALID, 0] {
            assert_eq!(ObjectType::from_raw(raw), None);
        }
    }

    #[test]
    fn object_filter_spells_any_as_none() {
        assert_eq!(ObjectType::raw_or_any(None), ffi::GIT_OBJECT_ANY);
        assert_eq!(ObjectType::from_raw(ObjectType::raw_or_any(None)), None);
    }

    #[test]
    fn not_an_object_type_reports_the_value() {
        let e = ObjectType::try_from(ffi::GIT_OBJECT_ANY).unwrap_err();
        assert_eq!(e.raw(), -2);
        assert_eq!(e.to_string(), "libgit2 value -2 is not an object type");
        assert!(e.source().is_none());
        let boxed: Box<dyn Error> = Box::new(e);
        assert_eq!(boxed.to_string(), e.to_string());
        assert_eq!(
            ObjectType::try_from(ffi::git_object_t::MIN)
                .unwrap_err()
                .to_string(),
            format!("libgit2 value {} is not an object type", i32::MIN)
        );
    }

    #[test]
    fn reference_variants_carry_their_c_values() {
        for (raw, kind) in REFERENCE_TYPES {
            assert_eq!(kind.raw(), raw);
            assert_eq!(ffi::git_reference_t::from(kind), raw);
            assert_eq!(ReferenceType::from_raw(raw), Some(kind));
            assert_eq!(ReferenceType::try_from(raw), Ok(kind));
        }
        assert_ne!(ReferenceType::Direct, ReferenceType::Symbolic);
    }

    #[test]
    fn reference_from_raw_refuses_every_other_value() {
        for raw in reference_probes() {
            let named = REFERENCE_TYPES
                .iter()
                .find(|(r, _)| *r == raw)
                .map(|(_, k)| *k);
            assert_eq!(ReferenceType::from_raw(raw), named, "{raw}");
            match ReferenceType::try_from(raw) {
                Ok(kind) => assert_eq!(Some(kind), named),
                Err(e) => {
                    assert_eq!(named, None);
                    assert_eq!(e.raw(), raw);
                }
            }
        }
        for raw in [ffi::GIT_REFERENCE_INVALID, ffi::GIT_REFERENCE_ALL] {
            assert_eq!(ReferenceType::from_raw(raw), None);
        }
    }

    #[test]
    fn not_a_reference_type_reports_the_value() {
        let e = ReferenceType::try_from(ffi::GIT_REFERENCE_ALL).unwrap_err();
        assert_eq!(e.raw(), 3);
        assert_eq!(e.to_string(), "libgit2 value 3 is not a reference type");
        assert!(e.source().is_none());
        let boxed: Box<dyn Error> = Box::new(e);
        assert_eq!(boxed.to_string(), e.to_string());
    }

    #[test]
    fn debug_names_the_variant() {
        assert_eq!(format!("{:?}", ObjectType::Commit), "Commit");
        assert_eq!(format!("{:?}", ReferenceType::Symbolic), "Symbolic");
        assert_eq!(
            format!("{:?}", ObjectType::try_from(5).unwrap_err()),
            "NotAnObjectType(5)"
        );
        assert_eq!(
            format!("{:?}", ReferenceType::try_from(0).unwrap_err()),
            "NotAReferenceType(0)"
        );
    }

    #[test]
    fn conversions_round_trip() {
        for v in [
            SubmoduleRecurse::No,
            SubmoduleRecurse::Yes,
            SubmoduleRecurse::OnDemand,
        ] {
            let raw = ffi::git_submodule_recurse_t::from(v);
            assert_eq!(SubmoduleRecurse::try_from(raw), Ok(v));
        }
        for v in [
            SubmoduleUpdate::Checkout,
            SubmoduleUpdate::Rebase,
            SubmoduleUpdate::Merge,
            SubmoduleUpdate::None,
        ] {
            let raw = ffi::git_submodule_update_t::from(v);
            assert_eq!(SubmoduleUpdate::try_from(raw), Ok(v));
        }
    }

    #[test]
    fn defaults_are_what_an_unconfigured_submodule_reports() {
        assert_eq!(SubmoduleRecurse::default(), SubmoduleRecurse::No);
        assert_eq!(SubmoduleUpdate::default(), SubmoduleUpdate::Checkout);
        assert_eq!(
            SubmoduleUpdate::from_raw(ffi::GIT_SUBMODULE_UPDATE_DEFAULT).unwrap_or_default(),
            SubmoduleUpdate::Checkout
        );
    }

    #[test]
    fn unnamed_values_report_themselves() {
        let e = SubmoduleRecurse::try_from(3).unwrap_err();
        assert_eq!(e.raw(), 3);
        assert_eq!(
            e.to_string(),
            "libgit2 submodule recurse setting 3 is not named by git2/types.h"
        );
        assert!(e.source().is_none());

        let e = SubmoduleUpdate::try_from(ffi::GIT_SUBMODULE_UPDATE_DEFAULT).unwrap_err();
        assert_eq!(e.raw(), 0);
        assert_eq!(
            e.to_string(),
            "libgit2 submodule update setting 0 has no SubmoduleUpdate variant"
        );
        let e = SubmoduleUpdate::try_from(u32::MAX).unwrap_err();
        assert_eq!(e.raw(), u32::MAX);
        assert!(e.source().is_none());
    }

    fn hash_of(time: &Time) -> u64 {
        let mut h = DefaultHasher::new();
        time.hash(&mut h);
        h.finish()
    }

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<Time>(), 16);
        assert_eq!(size_of::<TimeRef<'_>>(), size_of::<*const ffi::git_time>());
        assert_eq!(size_of::<TimeMut<'_>>(), size_of::<*const ffi::git_time>());
    }

    #[test]
    fn zeroed_is_epoch_without_sign() {
        let time = Time::zeroed();
        assert_eq!(time.as_ref().seconds(), 0);
        assert_eq!(time.as_ref().offset_minutes(), 0);
        assert_eq!(time.as_ref().sign(), 0);
    }

    #[test]
    fn new_signs_by_offset() {
        for (offset, sign) in [
            (i32::MIN, b'-'),
            (-1, b'-'),
            (0, b'+'),
            (1, b'+'),
            (i32::MAX, b'+'),
        ] {
            for seconds in [i64::MIN, -1, 0, 1_234_567_890, i64::MAX] {
                let time = Time::new(seconds, offset);
                let r = time.as_ref();
                assert_eq!(
                    (r.seconds(), r.offset_minutes(), r.sign()),
                    (seconds, offset, sign)
                );
            }
        }
    }

    #[test]
    fn setters_write_one_field_each() {
        let mut time = Time::new(10, -90);
        time.as_mut().set_seconds(-5);
        assert_eq!(time, {
            let mut t = Time::zeroed();
            let mut m = t.as_mut();
            m.set_seconds(-5);
            m.set_offset_minutes(-90);
            m.set_sign(b'-');
            t
        });
        // The sign is independent of the offset: `-0000`, and a sign byte libgit2
        // copied from a malformed zone.
        time.as_mut().set_offset_minutes(0);
        assert_eq!(time.as_ref().sign(), b'-');
        for sign in [0u8, b'x', 0x7F, 0x80, 0xFF] {
            time.as_mut().set_sign(sign);
            assert_eq!(time.as_ref().sign(), sign);
            assert_eq!(time.as_ref().offset_minutes(), 0);
            assert_eq!(time.as_ref().seconds(), -5);
        }
    }

    #[test]
    fn mut_handle_reborrows() {
        let mut time = Time::zeroed();
        let mut m = time.as_mut();
        m.as_mut().set_seconds(7);
        assert_eq!(m.as_ref().seconds(), 7);
        assert_eq!(
            format!("{m:?}"),
            format!(
                "{:?}",
                Time::from(ffi::git_time {
                    time: 7,
                    offset: 0,
                    sign: 0,
                })
            )
        );
    }

    #[test]
    fn raw_conversions_round_trip() {
        let raw = ffi::git_time {
            time: -42,
            offset: 330,
            sign: b'x'.cast_signed(),
        };
        let time = Time::from(raw);
        assert_eq!(time.as_ref().seconds(), -42);
        assert_eq!(time.as_ref().offset_minutes(), 330);
        assert_eq!(time.as_ref().sign(), b'x');
        let back = ffi::git_time::from(time);
        assert_eq!(
            (back.time, back.offset, back.sign),
            (raw.time, raw.offset, raw.sign)
        );
    }

    #[test]
    fn to_time_copies_the_fields() {
        let time = Time::new(1_700_000_000, 60);
        let copy = time.as_ref().to_time();
        assert_eq!(copy, time);
        assert_eq!(hash_of(&copy), hash_of(&time));
    }

    #[test]
    fn eq_and_hash_follow_every_field() {
        let base = Time::new(100, 60);
        let mut seconds = base;
        seconds.as_mut().set_seconds(101);
        let mut offset = base;
        offset.as_mut().set_offset_minutes(61);
        let mut sign = base;
        sign.as_mut().set_sign(b'-');
        for other in [seconds, offset, sign] {
            assert_ne!(base, other);
            assert_ne!(base.as_ref(), other.as_ref());
        }
        assert_eq!(base, Time::new(100, 60));
        assert_eq!(hash_of(&base), hash_of(&Time::new(100, 60)));
    }

    #[test]
    fn debug_shows_the_fields() {
        assert_eq!(
            format!("{:?}", Time::new(1, -60)),
            "Time { seconds: 1, offset_minutes: -60, sign: '-' }"
        );
    }

    const MODES: [(ffi::git_filemode_t, FileMode); 6] = [
        (0o000_000, FileMode::Unreadable),
        (0o040_000, FileMode::Tree),
        (0o100_644, FileMode::Blob),
        (0o100_755, FileMode::BlobExecutable),
        (0o120_000, FileMode::Link),
        (0o160_000, FileMode::Commit),
    ];

    #[test]
    fn variants_carry_the_octal_modes() {
        for (raw, mode) in MODES {
            assert_eq!(mode.raw(), raw);
            assert_eq!(ffi::git_filemode_t::from(mode), raw);
            assert_eq!(FileMode::from_raw(raw), Some(mode));
            assert_eq!(FileMode::try_from(raw), Ok(mode));
        }
    }

    #[test]
    fn unnamed_modes_keep_their_value() {
        for raw in [
            0o100_664,
            0o100_600,
            0o040_755,
            0o160_755,
            1,
            0o177_777,
            u32::MAX,
        ] {
            assert_eq!(FileMode::from_raw(raw), None);
            let err = FileMode::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(err, UnnamedFileMode(raw));
        }
    }

    #[test]
    fn unnamed_mode_displays_in_octal() {
        let err = FileMode::try_from(0o100_664).unwrap_err();
        assert_eq!(
            err.to_string(),
            "libgit2 file mode 0o100664 is not named by git2/types.h"
        );
        let boxed: Box<dyn Error> = Box::new(err);
        assert!(boxed.source().is_none());
    }

    #[test]
    fn modes_hash_and_compare_by_variant() {
        use std::collections::HashSet;
        let set: HashSet<FileMode> = MODES.iter().map(|&(_, m)| m).collect();
        assert_eq!(set.len(), MODES.len());
        assert_eq!(format!("{:?}", FileMode::BlobExecutable), "BlobExecutable");
    }

    /// `git_writestream`: `WriteStream`, `RustWriteStream` and their policies.
    mod write_streams {
        use core::ptr;
        use std::cell::RefCell;
        use std::rc::Rc;

        use super::super::*;

        /// What a writer saw, in order.
        #[derive(Clone, Debug, PartialEq, Eq)]
        enum Call {
            Write(Vec<u8>),
            Close,
            Drop,
        }

        /// A writer logging its calls into a log the test keeps, failing on request.
        #[derive(Debug)]
        struct Logged {
            log: Rc<RefCell<Vec<Call>>>,
            fail_write: Option<StreamError>,
            fail_close: Option<StreamError>,
        }

        impl Logged {
            fn new() -> (Self, Rc<RefCell<Vec<Call>>>) {
                let log = Rc::new(RefCell::new(Vec::new()));
                let writer = Self {
                    log: log.clone(),
                    fail_write: None,
                    fail_close: None,
                };
                (writer, log)
            }
        }

        impl StreamWriter for Logged {
            fn write(&mut self, data: &[u8]) -> Result<(), StreamError> {
                self.log.borrow_mut().push(Call::Write(data.to_vec()));
                self.fail_write.clone().map_or(Ok(()), Err)
            }

            fn close(&mut self) -> Result<(), StreamError> {
                self.log.borrow_mut().push(Call::Close);
                self.fail_close.clone().map_or(Ok(()), Err)
            }
        }

        impl Drop for Logged {
            fn drop(&mut self) {
                self.log.borrow_mut().push(Call::Drop);
            }
        }

        fn init() {
            assert!(runtime_pinned());
        }

        #[test]
        fn vec_writer_collects_writes_through_the_header() {
            let mut stream = RustWriteStream::new(Vec::new());
            stream.as_mut().write(b"one ").unwrap();
            stream.as_mut().write(b"").unwrap();
            stream.as_mut().write(b"two").unwrap();
            assert_eq!(stream.writer(), b"one two");
            stream.writer_mut().clear();
            stream.as_mut().write(b"three").unwrap();
            assert_eq!(
                format!("{stream:?}"),
                "RustWriteStream { writer: [116, 104, 114, 101, 101] }"
            );
            assert_eq!(stream.into_inner(), b"three");
        }

        #[test]
        fn borrowed_writer_is_kept_by_the_caller() {
            let mut out = Vec::new();
            {
                let mut stream = RustWriteStream::new(&mut out);
                stream.as_mut().write(b"kept").unwrap();
                // The default `close` accepts, and the stream accepts writes afterwards.
                assert_eq!(stream.writer_mut().close(), Ok(()));
                stream.as_mut().write(b"!").unwrap();
            }
            assert_eq!(out, b"kept!");
        }

        #[test]
        fn owner_drops_the_writer_once() {
            let (writer, log) = Logged::new();
            let mut stream = RustWriteStream::new(writer);
            stream.as_mut().write(b"x").unwrap();
            drop(stream);
            assert_eq!(*log.borrow(), [Call::Write(b"x".to_vec()), Call::Drop]);

            let (writer, log) = Logged::new();
            let writer = RustWriteStream::new(writer).into_inner();
            assert!(log.borrow().is_empty());
            drop(writer);
            assert_eq!(*log.borrow(), [Call::Drop]);
        }

        #[test]
        fn box_frees_through_the_member() {
            let (writer, log) = Logged::new();
            let mut stream = RustWriteStream::new(writer).into_box();
            stream.as_mut().write(b"abc").unwrap();
            drop(stream);
            assert_eq!(*log.borrow(), [Call::Write(b"abc".to_vec()), Call::Drop]);
        }

        #[test]
        fn close_runs_close_then_free() {
            let (writer, log) = Logged::new();
            let mut stream = RustWriteStream::new(writer).into_box();
            stream.as_mut().write(b"abc").unwrap();
            assert_eq!(WriteStream::close(stream), Ok(()));
            assert_eq!(
                *log.borrow(),
                [Call::Write(b"abc".to_vec()), Call::Close, Call::Drop]
            );
        }

        #[test]
        fn missing_free_member_is_left_alone() {
            let mut header = ffi::git_writestream::default();
            // SAFETY: a live header the test owns, with no `free` member to call.
            unsafe { stream_free(&raw mut header) };
        }

        #[test]
        fn handles_designate_the_header() {
            let mut stream = RustWriteStream::new(Vec::new());
            let header = stream.header();
            assert_eq!(stream.as_ref().as_ptr(), header.cast_const());
            assert_eq!(stream.as_mut().as_mut_ptr(), header);
            assert_eq!(header.cast::<Repr<Vec<u8>>>(), stream.raw.as_ptr());
            let boxed = stream.into_box();
            assert_eq!(boxed.as_c_ptr(), header);
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn writer_errors_cross_the_member_as_c_errors() {
            init();
            let (mut writer, log) = Logged::new();
            writer.fail_write = Some(
                StreamError::new(ErrorClass::Filter, c"write refused").with_code(ErrorCode::User),
            );
            writer.fail_close = Some(StreamError::new(ErrorClass::Os, c"close refused"));
            let mut stream = RustWriteStream::new(writer);

            let error = stream.as_mut().write(b"x").unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::User));
            assert_eq!(error.raw_code(), ffi::GIT_EUSER);
            assert_eq!(error.class(), Some(ErrorClass::Filter));
            assert_eq!(error.message(), c"write refused");
            // The member left the thread's error, as a C stream does.
            let last = last();
            assert_eq!(
                (last.class(), last.message()),
                (Some(ErrorClass::Filter), c"write refused")
            );

            let mut stream = stream.into_box();
            stream.as_mut().write(b"y").unwrap_err();
            let error = WriteStream::close(stream).unwrap_err();
            assert_eq!(
                (error.code(), error.class(), error.message()),
                (
                    Some(ErrorCode::Generic),
                    Some(ErrorClass::Os),
                    c"close refused"
                )
            );
            assert_eq!(
                *log.borrow(),
                [
                    Call::Write(b"x".to_vec()),
                    Call::Write(b"y".to_vec()),
                    Call::Close,
                    Call::Drop,
                ]
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn null_buffer_is_empty_only_when_len_is_zero() {
            init();
            let (writer, log) = Logged::new();
            let mut stream = RustWriteStream::new(writer);
            let header = stream.as_mut().as_mut_ptr();
            // SAFETY: the header of the live stream, reached by no other pointer; a NULL
            // buffer, which the member accepts.
            let rc = unsafe { write_with::<Logged>(header, ptr::null(), 0) };
            assert_eq!(rc, 0);
            // SAFETY: as above.
            let rc = unsafe { write_with::<Logged>(header, ptr::null(), 5) };
            assert_eq!(rc, ffi::GIT_ERROR);
            let last = last();
            assert_eq!(
                (last.class(), last.message()),
                (Some(ErrorClass::Invalid), c"NULL buffer written to stream")
            );
            assert_eq!(*log.borrow(), [Call::Write(Vec::new())]);
        }

        /// The typed stream lent to C as a filter target: libgit2 writes and closes it,
        /// never frees it, and the writer stays usable afterwards.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn typed_stream_lent_to_c_as_a_target() {
            init();
            let (writer, log) = Logged::new();
            let mut stream = RustWriteStream::new(writer);
            let data = b"lent to C";
            // SAFETY: no filters; `data` is live for the call; the target is the header of
            // a live stream, whose members accept the write and close libgit2 makes.
            let rc = unsafe {
                ffi::git_filter_list_stream_buffer(
                    ptr::null_mut(),
                    data.as_ptr().cast(),
                    data.len(),
                    stream.as_mut().as_mut_ptr(),
                )
            };
            assert_eq!(rc, 0);
            assert_eq!(*log.borrow(), [Call::Write(data.to_vec()), Call::Close]);
            // Closed by C, and written again: a Rust stream accepts any order.
            stream.as_mut().write(b"again").unwrap();
            drop(stream);
            assert_eq!(log.borrow().last(), Some(&Call::Drop));
        }

        #[test]
        fn stream_error_builds_and_formats() {
            let error = StreamError::new(ErrorClass::Odb, c"bad object");
            assert_eq!(error.code(), Some(ErrorCode::Generic));
            assert_eq!(error.raw_code(), -1);
            assert_eq!(
                (error.class(), error.raw_class()),
                (Some(ErrorClass::Odb), 9)
            );
            assert_eq!(error.to_string(), "bad object");
            assert_eq!(
                format!("{error:?}"),
                r#"StreamError { code: Generic, class: Odb, message: "bad object" }"#
            );
            // `Ok` would read as success in C.
            assert_eq!(error.clone().with_code(ErrorCode::Ok), error);
            let passthrough = error.clone().with_code(ErrorCode::Passthrough);
            assert_eq!(passthrough.raw_code(), ffi::GIT_PASSTHROUGH);
            assert_ne!(passthrough, error);
            let dyn_err: &dyn std::error::Error = &error;
            assert!(dyn_err.source().is_none());

            let unnamed = StreamError {
                code: -1000,
                class: 1000,
                message: c"path \xff".to_owned(),
            };
            assert_eq!((unnamed.code(), unnamed.class()), (None, None));
            assert_eq!(unnamed.to_string(), "path \u{fffd}");
            assert_eq!(
                format!("{unnamed:?}"),
                r#"StreamError { code: -1000, class: 1000, message: "path \xff" }"#
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn stream_error_from_another_wrapper() {
            init();
            set(ErrorClass::Index, c"from elsewhere");
            let error = StreamError::from(last());
            assert_eq!(
                (error.code(), error.class(), error.message()),
                (
                    Some(ErrorCode::Generic),
                    Some(ErrorClass::Index),
                    c"from elsewhere"
                )
            );
        }

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// Runs `body` alone in a child process, where nothing has initialised libgit2.
        fn isolated(test: &str, body: fn()) {
            let name = test.split_once("::").map_or(test, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                body();
                return;
            }
            let out = std::process::Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, name)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "child `{name}` failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// An allocator refusing every request, so `git_libgit2_init` fails.
        struct Refusing;

        // SAFETY: every request fails, which `Alloc` allows; nothing is ever allocated, so
        // nothing is resized or released.
        unsafe impl crate::libgit2::util::sys::alloc::Alloc for Refusing {
            fn malloc(_: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                None
            }

            unsafe fn realloc(
                _: Option<NonNull<u8>>,
                _: usize,
                _: Option<&CStr>,
                _: i32,
            ) -> Option<NonNull<u8>> {
                None
            }

            unsafe fn free(_: NonNull<u8>) {}
        }

        /// When libgit2 cannot be initialised, a writer's error is not stored (nothing
        /// reaches `errors.c`), but its code still comes back.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn writer_error_without_a_runtime_keeps_its_code() {
            isolated(
                concat!(
                    module_path!(),
                    "::writer_error_without_a_runtime_keeps_its_code"
                ),
                || {
                    let allocator = crate::libgit2::util::sys::alloc::Allocator::new::<Refusing>();
                    // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with
                    // every member set; libgit2 copies it before anything is allocated.
                    let rc = unsafe {
                        ffi::git_libgit2_opts(
                            ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                            allocator.as_ref().as_ptr(),
                        )
                    };
                    assert_eq!(rc, 0);
                    let (mut writer, _log) = Logged::new();
                    writer.fail_close =
                        Some(StreamError::new(ErrorClass::Filter, c"x").with_code(ErrorCode::User));
                    let mut stream = RustWriteStream::new(writer).into_box();
                    stream.as_mut().write(b"fine").unwrap();
                    let error = WriteStream::close(stream).unwrap_err();
                    assert!(!runtime_pinned());
                    assert_eq!(error.code(), Some(ErrorCode::User));
                    // `last` synthesises C's static error rather than call libgit2.
                    assert_eq!(error.message(), c"library has not been initialized");
                },
            );
        }

        #[test]
        fn auto_traits_follow_the_writer() {
            fn send<T: Send>() {}
            fn sync<T: Sync>() {}
            send::<RustWriteStream<Vec<u8>>>();
            sync::<RustWriteStream<Vec<u8>>>();
            send::<StreamError>();
            sync::<StreamError>();
            send::<WriteStreamFree>();
        }
    }

    const SUBMODULE_IGNORES: [(SubmoduleIgnore, ffi::git_submodule_ignore_t); 5] = [
        (SubmoduleIgnore::Unspecified, -1),
        (SubmoduleIgnore::None, 1),
        (SubmoduleIgnore::Untracked, 2),
        (SubmoduleIgnore::Dirty, 3),
        (SubmoduleIgnore::All, 4),
    ];

    /// Every variant converts to the value `git2/types.h` gives it, and back.
    #[test]
    fn submodule_ignore_named_values_round_trip() {
        for (variant, raw) in SUBMODULE_IGNORES {
            assert_eq!(variant.raw(), raw);
            assert_eq!(ffi::git_submodule_ignore_t::from(variant), raw);
            assert_eq!(SubmoduleIgnore::from_raw(raw), Some(variant));
            assert_eq!(SubmoduleIgnore::try_from(raw), Ok(variant));
        }
        assert_eq!(SubmoduleIgnore::default(), SubmoduleIgnore::Unspecified);
    }

    /// Every other value is rejected before it can become a variant, without panicking,
    /// and the error keeps it; an exhaustive sweep around the named range, plus the
    /// extremes.
    #[test]
    fn submodule_ignore_unnamed_values_are_rejected() {
        let named = |raw| SUBMODULE_IGNORES.iter().any(|&(_, r)| r == raw);
        for raw in (-300..300).chain([i32::MIN, i32::MIN + 1, i32::MAX - 1, i32::MAX]) {
            if named(raw) {
                continue;
            }
            assert_eq!(SubmoduleIgnore::from_raw(raw), None, "{raw}");
            let err = SubmoduleIgnore::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(
                err.to_string(),
                format!("libgit2 submodule ignore value {raw} is not named by git2/types.h")
            );
        }
        let zero: Box<dyn Error> = Box::new(UnnamedSubmoduleIgnore(0));
        assert!(zero.source().is_none());
    }

    #[test]
    fn submodule_ignore_layout_matches_c() {
        assert_eq!(
            size_of::<SubmoduleIgnore>(),
            size_of::<ffi::git_submodule_ignore_t>()
        );
        assert_eq!(
            size_of::<Option<SubmoduleIgnore>>(),
            size_of::<ffi::git_submodule_ignore_t>()
        );
    }

    const BRANCH_TYPES: [BranchType; 3] = [BranchType::Local, BranchType::Remote, BranchType::All];

    #[test]
    fn branch_raw_values_round_trip() {
        for kind in BRANCH_TYPES {
            assert_eq!(BranchType::from_raw(kind.raw()), Some(kind));
            assert_eq!(BranchType::try_from(kind.raw()), Ok(kind));
            assert_eq!(ffi::git_branch_t::from(kind), kind.raw());
        }
        assert_eq!(BranchType::Local.raw(), 1);
        assert_eq!(BranchType::Remote.raw(), 2);
        assert_eq!(BranchType::All.raw(), 3);
    }

    #[test]
    fn branch_unnamed_values_are_rejected() {
        for raw in [0, 4, 5, 7, 8, 0x8000_0000, u32::MAX] {
            assert_eq!(BranchType::from_raw(raw), None);
            let err = BranchType::try_from(raw).unwrap_err();
            assert_eq!(err.raw(), raw);
            assert_eq!(err.to_string(), format!("unnamed branch type {raw}"));
        }
    }

    #[test]
    fn branch_union_and_containment_follow_the_bits() {
        for a in BRANCH_TYPES {
            for b in BRANCH_TYPES {
                let union = a | b;
                assert_eq!(union.raw(), a.raw() | b.raw());
                assert_eq!(a.contains(b), a.raw() & b.raw() == b.raw());
                assert!(union.contains(a) && union.contains(b));
            }
            assert!(BranchType::All.contains(a));
            assert!(a.contains(a));
        }
        assert!(!BranchType::Local.contains(BranchType::Remote));
        assert!(!BranchType::Remote.contains(BranchType::All));
    }

    #[test]
    fn branch_enum_is_the_c_enum() {
        assert_eq!(size_of::<BranchType>(), size_of::<ffi::git_branch_t>());
        assert_eq!(align_of::<BranchType>(), align_of::<ffi::git_branch_t>());
    }

    mod signatures {
        use std::sync::atomic::Ordering::SeqCst;
        use std::sync::atomic::{AtomicBool, AtomicIsize};

        use super::super::*;
        use crate::libgit2::raw::{OdbDataBox, OdbDataStrFree};
        use crate::libgit2::util::sys::alloc::{Alloc, Allocator};

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// Runs `body` alone in a child process, where nothing has initialised libgit2.
        fn isolated(test: &str, body: fn()) {
            isolated_with(test, body, true);
        }

        /// As [`isolated`], with `LeakSanitizer` off in the child when `detect_leaks` is
        /// false: for bodies that drive libgit2 down a path on which it leaks.
        fn isolated_with(test: &str, body: fn(), detect_leaks: bool) {
            let name = test.split_once("::").map_or(test, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                body();
                return;
            }
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, name);
            if !detect_leaks {
                let inherited = std::env::var("ASAN_OPTIONS").unwrap_or_default();
                let separator = if inherited.is_empty() { "" } else { ":" };
                command.env(
                    "ASAN_OPTIONS",
                    format!("{inherited}{separator}detect_leaks=0"),
                );
            }
            let out = command.output().unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "child `{name}` failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        /// Refuses every request while set.
        static REFUSE_ALL: AtomicBool = AtomicBool::new(false);
        /// Requests still granted before one is refused; negative grants all.
        static GRANTS: AtomicIsize = AtomicIsize::new(-1);

        unsafe extern "C" {
            fn malloc(n: usize) -> *mut core::ffi::c_void;
            fn realloc(p: *mut core::ffi::c_void, n: usize) -> *mut core::ffi::c_void;
            fn free(p: *mut core::ffi::c_void);
        }

        /// libc's allocator, refusing everything under `REFUSE_ALL`, and the one request
        /// `GRANTS` counts down to.
        struct Failing;

        impl Failing {
            fn refuse() -> bool {
                REFUSE_ALL.load(SeqCst) || GRANTS.fetch_sub(1, SeqCst) == 0
            }
        }

        // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
        unsafe impl Alloc for Failing {
            fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                if Self::refuse() {
                    return None;
                }
                // SAFETY: plain libc allocation.
                NonNull::new(unsafe { malloc(len) }.cast())
            }

            unsafe fn realloc(
                p: Option<NonNull<u8>>,
                len: usize,
                _: Option<&CStr>,
                _: i32,
            ) -> Option<NonNull<u8>> {
                if Self::refuse() {
                    return None;
                }
                let p = p.map_or(ptr::null_mut(), |p| p.as_ptr().cast());
                // SAFETY: `p` is NULL or a live libc block the caller gives up on success.
                NonNull::new(unsafe { realloc(p, len) }.cast())
            }

            unsafe fn free(p: NonNull<u8>) {
                // SAFETY: `p` is a live libc block the caller gives up.
                unsafe { free(p.as_ptr().cast()) }
            }
        }

        /// Installs [`Failing`] as libgit2's allocator, before anything is allocated.
        fn install_failing() {
            let allocator = Allocator::new::<Failing>();
            // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with
            // every member set; libgit2 copies it before anything is allocated.
            let rc = unsafe {
                ffi::git_libgit2_opts(
                    ffi::GIT_OPT_SET_ALLOCATOR.cast_signed(),
                    allocator.as_ref().as_ptr(),
                )
            };
            assert_eq!(rc, 0);
        }

        /// A signature holding `name` and `email` as they are, NULL included, as
        /// libgit2's parser leaves one after losing a string; built in libgit2's
        /// allocator and released by `git_signature_free`.
        fn raw_signature(name: Option<&CStr>, email: Option<&CStr>) -> SignatureBox {
            let dup = |s: Option<&CStr>| {
                s.map_or(ptr::null_mut(), |s| {
                    OdbDataStrFree::dup(s).unwrap().into_raw()
                })
            };
            let mut storage = OdbDataBox::<Signature>::new_uninit().unwrap();
            // SAFETY: `storage` is fresh, writable storage for one `git_signature`.
            unsafe {
                storage.as_c_ptr().write(ffi::git_signature {
                    name: dup(name),
                    email: dup(email),
                    when: Time::new(5, -90).into(),
                });
            };
            // SAFETY: the storage now holds a signature whose strings are NULL or libgit2-allocated
            // strings it owns.
            let signature = unsafe { storage.assume_init() };
            // SAFETY: `git_signature_free` releases those strings with the struct, through the
            // allocator `OdbDataFree` used.
            unsafe { signature.with_policy(SignatureFree) }.0
        }

        #[test]
        fn layout_matches_c() {
            assert_eq!(size_of::<Signature>(), size_of::<ffi::git_signature>());
            assert_eq!(align_of::<Signature>(), align_of::<ffi::git_signature>());
            assert_eq!(
                size_of::<SignatureRef<'_>>(),
                size_of::<*const ffi::git_signature>()
            );
            assert_eq!(
                size_of::<SignatureMut<'_>>(),
                size_of::<*const ffi::git_signature>()
            );
            assert_eq!(
                size_of::<SignatureBox>(),
                size_of::<*mut ffi::git_signature>()
            );
            assert_eq!(
                size_of::<Option<SignatureBox>>(),
                size_of::<*mut ffi::git_signature>()
            );
        }

        /// A view lends its strings in place, never copies them, and reads its time back.
        #[test]
        fn view_reads_its_borrowed_strings() {
            let name = c"A U Thor".to_owned();
            let email = c"author@example.com".to_owned();
            let view = SignatureView::new(&name, &email, Time::new(1_700_000_000, -90));
            let r = view.as_ref();
            assert_eq!(r.name().unwrap().as_ptr(), name.as_ptr());
            assert_eq!(r.email().unwrap().as_ptr(), email.as_ptr());
            assert_eq!(r.when().to_time(), Time::new(1_700_000_000, -90));
            assert_eq!(r.when().sign(), b'-');
            let copy = view.clone();
            assert_eq!(copy.as_ref(), r);
            assert_eq!(copy.as_ref().name().unwrap().as_ptr(), name.as_ptr());
            assert_eq!(
                format!("{view:?}"),
                r#"Signature { name: Some("A U Thor"), email: Some("author@example.com"), when: Time { seconds: 1700000000, offset_minutes: -90, sign: '-' } }"#
            );
        }

        /// Equality is field by field: strings by bytes, times by their three fields.
        #[test]
        fn views_compare_field_by_field() {
            let base = SignatureView::new(c"A", c"a@b", Time::new(1, 60));
            assert_eq!(
                base.as_ref(),
                SignatureView::new(c"A", c"a@b", Time::new(1, 60)).as_ref()
            );
            let different = [
                SignatureView::new(c"B", c"a@b", Time::new(1, 60)),
                SignatureView::new(c"A", c"a@c", Time::new(1, 60)),
                SignatureView::new(c"A", c"a@b", Time::new(2, 60)),
                SignatureView::new(c"A", c"a@b", Time::new(1, -60)),
            ];
            for other in &different {
                assert_ne!(base.as_ref(), other.as_ref(), "{other:?}");
            }
            let mut signed = Time::new(1, 60);
            signed.as_mut().set_sign(b'x');
            assert_ne!(
                base.as_ref(),
                SignatureView::new(c"A", c"a@b", signed).as_ref()
            );
        }

        #[test]
        fn auto_traits() {
            fn send_sync<T: Send + Sync>() {}
            send_sync::<SignatureBox>();
            send_sync::<SignatureView<'static>>();
            send_sync::<SignatureRef<'static>>();
            send_sync::<SignatureMut<'static>>();
        }

        /// The replacement setters move the new string in and the old one out, without
        /// copying either.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn replacing_moves_the_strings() {
            let mut sig = Signature::new(c"A", c"a@b", 7, 30).unwrap();
            let name = OdbDataStrFree::dup(c"B").unwrap();
            let name_at = name.as_ptr();
            let old_at = sig.as_ref().name().unwrap().as_ptr();
            let old = sig.as_mut().replace_name(name).unwrap();
            assert_eq!(old.as_ptr(), old_at);
            assert_eq!(old.to_str(), Ok("A"));
            assert_eq!(sig.as_ref().name().unwrap().as_ptr(), name_at);
            let email = OdbDataStrFree::dup(c"b@c").unwrap();
            let email_at = email.as_ptr();
            sig.as_mut().set_email(email);
            assert_eq!(sig.as_ref().email().unwrap().as_ptr(), email_at);
            assert_eq!(sig.as_ref().email(), Some(c"b@c"));
            assert_eq!(sig.as_ref().when().to_time(), Time::new(7, 30));
        }

        /// A signature without its strings reads them as `None`, takes new ones, refuses
        /// to be copied (C would `strdup(NULL)`), and is freed by `git_signature_free`.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn lost_strings_read_as_none_and_refuse_copies() {
            for (name, email) in [(None, Some(c"a@b")), (Some(c"A"), None), (None, None)] {
                let mut sig = raw_signature(name, email);
                let r = sig.as_ref();
                assert_eq!((r.name(), r.email()), (name, email));
                assert_eq!(r.when().to_time(), Time::new(5, -90));
                assert!(sig.try_clone().is_none());
                let error = sig.as_ref().to_signature().unwrap_err();
                assert_eq!(error.class(), Some(ErrorClass::Invalid));
                assert_eq!(
                    error.message(),
                    c"failed to copy signature - missing name or email"
                );
                assert!(format!("{:?}", sig.as_ref()).contains("None"));

                // Filling the lost strings makes it whole, and copyable.
                let lost_name = sig
                    .as_mut()
                    .replace_name(OdbDataStrFree::dup(c"N").unwrap());
                assert_eq!(lost_name.as_ref().map(ffibox::CStrBox::as_c_str), name);
                if sig.as_ref().email().is_none() {
                    sig.as_mut().set_email(OdbDataStrFree::dup(c"n@m").unwrap());
                }
                let copy = sig.try_clone().unwrap();
                assert_eq!(copy.as_ref(), sig.as_ref());
            }
        }

        /// `git_signature_from_buffer` reports success when it cannot copy the name or
        /// the email, leaving it NULL; `Signature::parse` refuses that signature with the
        /// out-of-memory error, and frees what C built (`LSan`, in the asan variant).
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn parse_refuses_a_signature_libgit2_lost_a_string_of() {
            isolated(
                concat!(
                    module_path!(),
                    "::parse_refuses_a_signature_libgit2_lost_a_string_of"
                ),
                || {
                    install_failing();
                    assert!(runtime_pinned());
                    // Allocate the thread's error state before allocations start failing.
                    set(ErrorClass::Invalid, c"primed");
                    let line = c"A U Thor <a@b> 12 +0100";

                    // The C defect: one request granted (the struct), the name refused.
                    GRANTS.store(1, SeqCst);
                    let mut out = ptr::null_mut();
                    // SAFETY: a writable slot and a NUL-terminated line, read for the call.
                    let rc = unsafe { ffi::git_signature_from_buffer(&raw mut out, line.as_ptr()) };
                    GRANTS.store(-1, SeqCst);
                    assert_eq!(rc, 0);
                    // SAFETY: `out` is the signature C returned, live until it is freed below.
                    assert!(unsafe { (*out).name }.is_null());
                    // SAFETY: as above.
                    assert!(!unsafe { (*out).email }.is_null());
                    // SAFETY: freed once, after its last read.
                    unsafe { ffi::git_signature_free(out) };

                    // The struct, the name, then the email refused: always an error.
                    for grants in 0..3 {
                        set(ErrorClass::Invalid, c"primed");
                        GRANTS.store(grants, SeqCst);
                        let error = Signature::parse(line).unwrap_err();
                        GRANTS.store(-1, SeqCst);
                        assert_eq!(error.class(), Some(ErrorClass::NoMemory), "{grants}");
                        assert_eq!(error.message(), c"Out of memory");
                    }
                    let sig = Signature::parse(line).unwrap();
                    assert_eq!(sig.as_ref().name(), Some(c"A U Thor"));

                    // A malformed timezone makes the parser set a conversion error of its
                    // own after the lost string, and still succeed: the refusal reports
                    // the allocation failure, not that later, stale error.
                    for malformed in [c"A U Thor <a@b> 12 +abc", c"A U Thor <a@b> 12 +99999999999"]
                    {
                        set(ErrorClass::Invalid, c"primed");
                        GRANTS.store(1, SeqCst);
                        let error = Signature::parse(malformed).unwrap_err();
                        GRANTS.store(-1, SeqCst);
                        assert_eq!(error.class(), Some(ErrorClass::NoMemory), "{malformed:?}");
                        assert_eq!(error.message(), c"Out of memory", "{malformed:?}");
                    }

                    // A copy that runs out of memory fails cleanly. C leaks its partial
                    // copy, so only the struct itself is refused here.
                    set(ErrorClass::Invalid, c"primed");
                    GRANTS.store(0, SeqCst);
                    assert!(sig.try_clone().is_none());
                    GRANTS.store(0, SeqCst);
                    let error = sig.as_ref().to_signature().unwrap_err();
                    GRANTS.store(-1, SeqCst);
                    assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                },
            );
        }

        /// `git_signature_new` fails with the out-of-memory error at each of its three
        /// allocations (the struct, the trimmed name, the trimmed email), and
        /// `git_signature_dup` at each of its two string copies; `Signature::new`,
        /// `try_clone` and `to_signature` report it, and the next request succeeds.
        ///
        /// libgit2 leaks the half-built struct (and the name) when a string allocation
        /// fails, so the child runs without `LeakSanitizer`.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn constructors_report_each_allocation_failure() {
            isolated_with(
                concat!(
                    module_path!(),
                    "::constructors_report_each_allocation_failure"
                ),
                || {
                    install_failing();
                    assert!(runtime_pinned());
                    let expect_oom = |error: ErrorBuf, grants: isize| {
                        assert_eq!(error.class(), Some(ErrorClass::NoMemory), "{grants}");
                        assert_eq!(error.message(), c"Out of memory", "{grants}");
                    };

                    // The struct, the name, then the email refused.
                    for grants in 0..3 {
                        // Allocate the thread's error state before requests start failing.
                        set(ErrorClass::Invalid, c"primed");
                        GRANTS.store(grants, SeqCst);
                        let result = Signature::new(c" A ", c"a@b", 1, 60);
                        GRANTS.store(-1, SeqCst);
                        expect_oom(result.unwrap_err(), grants);
                    }
                    let sig = Signature::new(c" A ", c"a@b", 1, 60).unwrap();
                    assert_eq!(sig.as_ref().name(), Some(c"A"));

                    // The copy's name, then its email refused (its struct is refused by
                    // `parse_refuses_a_signature_libgit2_lost_a_string_of`).
                    for grants in 1..3 {
                        set(ErrorClass::Invalid, c"primed");
                        GRANTS.store(grants, SeqCst);
                        let clone = sig.try_clone();
                        GRANTS.store(-1, SeqCst);
                        assert!(clone.is_none(), "{grants}");

                        set(ErrorClass::Invalid, c"primed");
                        GRANTS.store(grants, SeqCst);
                        let copy = sig.as_ref().to_signature();
                        GRANTS.store(-1, SeqCst);
                        expect_oom(copy.unwrap_err(), grants);
                    }
                    let copy = sig.as_ref().to_signature().unwrap();
                    assert_eq!(copy.as_ref(), sig.as_ref());
                    assert_eq!(sig.try_clone().unwrap().as_ref(), sig.as_ref());
                },
                false,
            );
        }

        /// When libgit2 cannot be initialised, every constructor and copy reports C's
        /// static "not initialized" error without calling it; a view still works.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn constructors_without_runtime_fail_cleanly() {
            isolated(
                concat!(
                    module_path!(),
                    "::constructors_without_runtime_fail_cleanly"
                ),
                || {
                    install_failing();
                    REFUSE_ALL.store(true, SeqCst);
                    let view = SignatureView::new(c"A", c"a@b", Time::new(0, 0));
                    let errors = [
                        Signature::new(c"A", c"a@b", 0, 0).unwrap_err(),
                        Signature::now(c"A", c"a@b").unwrap_err(),
                        Signature::parse(c"A <a@b> 0 +0000").unwrap_err(),
                        view.as_ref().to_signature().unwrap_err(),
                    ];
                    REFUSE_ALL.store(false, SeqCst);
                    assert!(!runtime_pinned(), "libgit2 could not initialise");
                    for e in errors {
                        assert_eq!(e.class(), Some(ErrorClass::Invalid));
                        assert_eq!(e.message(), c"library has not been initialized");
                    }
                    // SAFETY: the view's live signature, which the adapter only reads; it
                    // refuses before calling libgit2.
                    let copy = unsafe { signature_dup(view.as_ref().as_ptr().cast_mut()) };
                    assert!(copy.is_null());
                    assert_eq!(view.as_ref().name(), Some(c"A"));
                },
            );
        }
    }
}
