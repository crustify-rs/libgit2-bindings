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
//! The submodule settings ([`SubmoduleRecurse`], [`SubmoduleUpdate`]) are fieldless
//! enums over their C values, converted fallibly from a raw one.
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
//! Times compare as libgit2 compares signatures' times: equal when all three fields
//! are, whatever the struct's padding holds.
//!
//! A [`FileMode`] is a tree entry's `git_filemode_t`. A parsed tree keeps whatever
//! 16-bit mode its object spells, so a raw mode is converted fallibly
//! ([`FileMode::from_raw`]) and never transmuted.

use core::ffi::c_char;
use core::fmt;
use core::hash::{Hash, Hasher};
use core::marker::PhantomData;
use core::ptr::{addr_of, addr_of_mut};

use ffibox::{CZeroable, define_ctype};

use crate::ffi;

/// The type of a Git object: what its content is, which fixes how libgit2 parses it.
///
/// libgit2 answers `GIT_OBJECT_INVALID` where it has no type, and its lookups take
/// `GIT_OBJECT_ANY` to accept any type ([`raw_or_any`](Self::raw_or_any));
/// [`from_raw`](Self::from_raw) gives `None` for both, and for every value
/// `git_object_type_is_valid` refuses.
///
/// Wraps: git_object_t
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
    pub const fn raw(self) -> ffi::git_object_t {
        self as ffi::git_object_t
    }

    /// The C value of a type filter, as lookups and peels take it: `kind`'s value, or
    /// `GIT_OBJECT_ANY` for `None`, which accepts an object of any type.
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
/// Wraps: git_reference_t
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
    pub const fn from_raw(raw: ffi::git_reference_t) -> Option<Self> {
        match raw {
            ffi::GIT_REFERENCE_DIRECT => Some(Self::Direct),
            ffi::GIT_REFERENCE_SYMBOLIC => Some(Self::Symbolic),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions return it.
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
/// Wraps: git_submodule_recurse_t
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
    pub const fn from_raw(raw: ffi::git_submodule_recurse_t) -> Option<Self> {
        match raw {
            ffi::GIT_SUBMODULE_RECURSE_NO => Some(Self::No),
            ffi::GIT_SUBMODULE_RECURSE_YES => Some(Self::Yes),
            ffi::GIT_SUBMODULE_RECURSE_ONDEMAND => Some(Self::OnDemand),
            _ => None,
        }
    }

    /// The C value, as libgit2's functions take and return it.
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
/// Wraps: git_submodule_update_t
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
/// Wraps: git_filemode_t
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
    Time, TimeRef, TimeMut, ffi::git_time
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
    /// otherwise. Any value is accepted, as libgit2 accepts it.
    ///
    /// Handwritten
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

impl<'a> TimeRef<'a> {
    /// Seconds since the epoch, 1970-01-01T00:00:00Z; negative before it.
    ///
    /// Field: git_time.time
    pub fn seconds(&self) -> i64 {
        // SAFETY: a read through the shared handle's pointer to a live `git_time`; any
        // value is a valid `i64`.
        unsafe { addr_of!((*self.as_ptr()).time).read() }
    }

    /// The timezone's offset from UTC in minutes, negative west of it. libgit2 parses
    /// at most 14 hours and 59 minutes either way, but stores whatever it is given.
    ///
    /// Field: git_time.offset
    pub fn offset_minutes(&self) -> i32 {
        // SAFETY: a read through the shared handle's pointer to a live `git_time`; any
        // value is a valid `i32`.
        unsafe { addr_of!((*self.as_ptr()).offset).read() }
    }

    /// The sign character the offset was written with, as a byte: `b'+'` or `b'-'`
    /// for every time libgit2 builds, 0 where a parsed signature had no valid
    /// timezone, or whatever character a malformed one started with. `b'-'` with an
    /// offset of 0 is git's questionable `-0000`.
    ///
    /// Field: git_time.sign
    pub fn sign(&self) -> u8 {
        // SAFETY: a read through the shared handle's pointer to a live `git_time`; any
        // `char` is a valid byte.
        unsafe { addr_of!((*self.as_ptr()).sign).read() as u8 }
    }

    /// A copy of the time, as a value of its own.
    pub fn to_time(&self) -> Time {
        Time::from(ffi::git_time {
            time: self.seconds(),
            offset: self.offset_minutes(),
            sign: self.sign() as c_char,
        })
    }
}

impl TimeMut<'_> {
    /// Replaces the seconds since the epoch.
    ///
    /// Field: git_time.time
    pub fn set_seconds(&mut self, seconds: i64) {
        // SAFETY: a write through the exclusive handle's pointer to a live `git_time`;
        // the field is a plain integer.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).time).write(seconds) }
    }

    /// Replaces the offset from UTC in minutes. The sign character is left as it is.
    ///
    /// Field: git_time.offset
    pub fn set_offset_minutes(&mut self, offset_minutes: i32) {
        // SAFETY: a write through the exclusive handle's pointer to a live `git_time`;
        // the field is a plain integer.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).offset).write(offset_minutes) }
    }

    /// Replaces the sign character. Any byte is accepted, as libgit2 accepts it.
    ///
    /// Field: git_time.sign
    pub fn set_sign(&mut self, sign: u8) {
        // SAFETY: a write through the exclusive handle's pointer to a live `git_time`;
        // the field is a plain `char`, which every byte fits.
        unsafe { addr_of_mut!((*self.as_mut_ptr()).sign).write(sign as c_char) }
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

#[cfg(test)]
mod unit_tests {
    use super::*;
    use std::collections::HashSet;
    use std::collections::hash_map::DefaultHasher;
    use std::error::Error;

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
        for sign in [0u8, b'x', 0x7f, 0x80, 0xff] {
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
            sign: b'x' as c_char,
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
        (0o000000, FileMode::Unreadable),
        (0o040000, FileMode::Tree),
        (0o100644, FileMode::Blob),
        (0o100755, FileMode::BlobExecutable),
        (0o120000, FileMode::Link),
        (0o160000, FileMode::Commit),
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
            0o100664,
            0o100600,
            0o040755,
            0o160755,
            1,
            0o177777,
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
        let err = FileMode::try_from(0o100664).unwrap_err();
        assert_eq!(
            err.to_string(),
            "libgit2 file mode 0o100664 is not named by git2/types.h"
        );
        let boxed: Box<dyn std::error::Error> = Box::new(err);
        assert!(boxed.source().is_none());
    }

    #[test]
    fn modes_hash_and_compare_by_variant() {
        use std::collections::HashSet;
        let set: HashSet<FileMode> = MODES.iter().map(|&(_, m)| m).collect();
        assert_eq!(set.len(), MODES.len());
        assert_eq!(format!("{:?}", FileMode::BlobExecutable), "BlobExecutable");
    }
}
