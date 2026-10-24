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

use crate::ffi;

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
pub struct UnnamedSubmoduleIgnore(pub(crate) ffi::git_submodule_ignore_t);

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
