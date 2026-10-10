//! Safe wrappers over `include/git2/repository.h`.
//!
//! # Layout items and open flags
//!
//! An [`Item`] is a `git_repository_item_t`: one file or directory of a repository's
//! layout, which `git_repository_item_path` resolves against the repository's git,
//! common or working directory. libgit2 indexes a static table of
//! `GIT_REPOSITORY_ITEM__LAST` entries with the value unchecked, so the sentinel and
//! every value past it read beyond that table: [`Item`] has no variant for them, and
//! [`Item::from_raw`] refuses them.
//!
//! An [`OpenFlag`] is a `git_repository_open_flag_t`, one bit of the `unsigned int`
//! that `git_repository_open_ext` takes; an [`OpenFlags`] set ORs them. libgit2 only
//! tests the bits it names and ignores the others, so a set keeps any bit it is given
//! ([`OpenFlags::from_bits_retain`]) and passes it through unchanged.
//!
//! # Initialisation options
//!
//! An [`InitOptions`] is a `git_repository_init_options`: the [`InitFlags`],
//! [`InitMode`], strings, object id type and reference storage that
//! `git_repository_init_ext` creates a repository with. libgit2 never allocates or
//! releases one: the caller fills it with `git_repository_init_options_init`
//! ([`InitOptions::new`]) and keeps it, and libgit2 only reads its strings during the
//! call, never keeping or freeing them. So Rust holds the options by value and borrows
//! the strings for `'o`, the getters on [`InitOptionsRef`] and the setters on
//! [`InitOptionsMut`], which is invariant in `'o`. The object id type and reference
//! storage are typed, `None` for the zero default: libgit2 does not check them, and
//! creates a repository it cannot open from a value it does not name.
//!
//! `git_repository_init_ext` writes to its options, ORing internal bits into their
//! flags, so it takes them exclusively ([`InitOptionsMut`]).
//!
//! # Repository state
//!
//! A [`RepositoryState`] is a `git_repository_state_t`: the multi-step operation a
//! repository is in the middle of, as `git_repository_state` reads it from the marker
//! files git leaves in the git directory. Its one producer returns it as an `int`, and
//! `-1` when it cannot build the path it probes, so a raw value is converted fallibly
//! ([`RepositoryState::from_raw`] and `TryFrom`) and never transmuted.
//!
//! # The repository
//!
//! A [`Repository`] is the opaque `git_repository`, which libgit2 allocates and frees
//! with no count: one [`RepositoryBox`] owns it, built by the constructors on
//! [`Repository`], which report a failure as a [`RepositoryError`].

use core::ffi::{CStr, c_int, c_uint};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::ops::{BitOr, BitOrAssign};
use core::ptr::{self, NonNull, addr_of, addr_of_mut};

use ffibox::{CBorrowedPtr, CBox, CCell, CNew, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::oid::oid::OidType;
use crate::libgit2::refs::refdb::RefdbType;
use crate::libgit2::repository::sys::repository::NewOptions;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::runtime_pinned;

/// One option of `git_repository_init_ext`, a single bit of an [`InitFlags`] set
/// (`git_repository_init_flag_t`). Each is off by default.
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant: `0`,
/// a combination of bits, or a bit this version of libgit2's header does not name.
///
/// Wraps: git_repository_init_flag_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum InitFlag {
    /// Create a bare repository, with no working directory.
    Bare = ffi::GIT_REPOSITORY_INIT_BARE,
    /// Fail with `GIT_EEXISTS` if the path already holds a repository, rather than
    /// reinitialising it.
    NoReinit = ffi::GIT_REPOSITORY_INIT_NO_REINIT,
    /// Create the last component of the repository and working directory paths as
    /// needed; the `.git` directory itself is always created.
    Mkdir = ffi::GIT_REPOSITORY_INIT_MKDIR,
    /// Create every missing component of the repository and working directory paths.
    ///
    /// Its bit is `GIT_REPOSITORY_OPEN_FROM_ENV`'s too, and `git_repository_init_ext`
    /// tests that open flag on the same `flags`: with this flag set, init also honours
    /// `$GIT_COMMON_DIR` when deciding whether the path already holds a repository
    /// (`lookup_commondir`). It also passes the flag to its symlink probe, which on
    /// Windows alone reads `core.symlinks` from the global and system configuration
    /// at the environment's paths; elsewhere that probe ignores it, and init reads no
    /// configuration from the environment.
    Mkpath = ffi::GIT_REPOSITORY_INIT_MKPATH,
    /// Initialise from an external template: the options' template path, else the
    /// `init.templatedir` setting, else the system's template directory, rather than
    /// libgit2's built-in one.
    ExternalTemplate = ffi::GIT_REPOSITORY_INIT_EXTERNAL_TEMPLATE,
    /// With a working directory away from the repository, record the repository's
    /// location in the `.git` file and `core.worktree` as relative paths.
    RelativeGitlink = ffi::GIT_REPOSITORY_INIT_RELATIVE_GITLINK,
}

impl InitFlag {
    /// Every variant, in ascending bit order.
    pub const ALL: [InitFlag; 6] = [
        Self::Bare,
        Self::NoReinit,
        Self::Mkdir,
        Self::Mkpath,
        Self::ExternalTemplate,
        Self::RelativeGitlink,
    ];

    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_repository_init_flag_t) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as ffi::git_repository_init_flag_t == raw {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// The C value: the flag's bit.
    pub const fn raw(self) -> ffi::git_repository_init_flag_t {
        self as ffi::git_repository_init_flag_t
    }
}

impl TryFrom<ffi::git_repository_init_flag_t> for InitFlag {
    type Error = UnnamedInitFlag;

    fn try_from(raw: ffi::git_repository_init_flag_t) -> Result<Self, UnnamedInitFlag> {
        Self::from_raw(raw).ok_or(UnnamedInitFlag(raw))
    }
}

impl From<InitFlag> for ffi::git_repository_init_flag_t {
    fn from(value: InitFlag) -> ffi::git_repository_init_flag_t {
        value.raw()
    }
}

// An `InitFlag` passes where C takes a `git_repository_init_flag_t`.
const _: () = assert!(
    size_of::<InitFlag>() == size_of::<ffi::git_repository_init_flag_t>()
        && align_of::<InitFlag>() == align_of::<ffi::git_repository_init_flag_t>()
);

/// An init flag value that this version of libgit2's header does not name: `0`, a
/// combination of flags, or another bit. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedInitFlag(ffi::git_repository_init_flag_t);

impl UnnamedInitFlag {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_repository_init_flag_t {
        self.0
    }
}

impl fmt::Display for UnnamedInitFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 repository init flag {:#x} is not named by git2/repository.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedInitFlag {}

/// A set of [`InitFlag`]s: the `uint32_t` `flags` of `git_repository_init_options`,
/// OR-ing `GIT_REPOSITORY_INIT_*` bits. The default is the empty set, as
/// `git_repository_init_options_init` leaves it.
///
/// libgit2 reads bits no [`InitFlag`] names too: `git_repository_init_ext` ORs its
/// own internal bits (`1 << 16` for a path ending in `.git`, `1 << 17` for a working
/// directory that is its parent, `1 << 18` for a reinitialisation) into the options
/// it is given, and reads them back on the next call with those options.
/// [`from_bits_retain`](Self::from_bits_retain) keeps every bit, so
/// [`bits`](Self::bits) returns what C produced; [`from_bits_truncate`](Self::from_bits_truncate)
/// keeps the named ones only. [`iter`](Self::iter) and [`contains`](Self::contains)
/// see the named flags, and [`unnamed_bits`](Self::unnamed_bits) the rest.
///
/// Wraps: git_repository_init_flag_t
/// Handwritten
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct InitFlags(u32);

impl InitFlags {
    /// The bits of every [`InitFlag`].
    const NAMED: u32 = {
        let mut bits = 0;
        let mut i = 0;
        while i < InitFlag::ALL.len() {
            bits |= InitFlag::ALL[i].raw();
            i += 1;
        }
        bits
    };

    /// The set with no flag.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The set of every bit of `bits`, as the options' `flags` hold them; bits that no
    /// [`InitFlag`] names are kept.
    pub const fn from_bits_retain(bits: u32) -> Self {
        Self(bits)
    }

    /// The set of the bits of `bits` that an [`InitFlag`] names; the rest are dropped.
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & Self::NAMED)
    }

    /// The bits of the set, as the options' `flags` take them.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The bits of the set that no [`InitFlag`] names.
    pub const fn unnamed_bits(self) -> u32 {
        self.0 & !Self::NAMED
    }

    /// Whether the set has no bit, named or not.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `flag` is in the set.
    pub const fn contains(self, flag: InitFlag) -> bool {
        self.0 & flag.raw() != 0
    }

    /// Adds `flag` to the set.
    pub fn insert(&mut self, flag: InitFlag) {
        self.0 |= flag.raw();
    }

    /// Removes `flag` from the set.
    pub fn remove(&mut self, flag: InitFlag) {
        self.0 &= !flag.raw();
    }

    /// The named flags in the set, in ascending bit order.
    pub fn iter(self) -> InitFlagsIter {
        InitFlagsIter { set: self, next: 0 }
    }
}

impl From<InitFlag> for InitFlags {
    fn from(flag: InitFlag) -> Self {
        Self(flag.raw())
    }
}

impl BitOr for InitFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<InitFlag> for InitFlags {
    type Output = Self;

    fn bitor(self, rhs: InitFlag) -> Self {
        self | Self::from(rhs)
    }
}

impl BitOr for InitFlag {
    type Output = InitFlags;

    fn bitor(self, rhs: Self) -> InitFlags {
        InitFlags::from(self) | rhs
    }
}

impl BitOrAssign for InitFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitOrAssign<InitFlag> for InitFlags {
    fn bitor_assign(&mut self, rhs: InitFlag) {
        self.insert(rhs);
    }
}

impl FromIterator<InitFlag> for InitFlags {
    fn from_iter<I: IntoIterator<Item = InitFlag>>(iter: I) -> Self {
        let mut set = Self::empty();
        set.extend(iter);
        set
    }
}

impl Extend<InitFlag> for InitFlags {
    fn extend<I: IntoIterator<Item = InitFlag>>(&mut self, iter: I) {
        for flag in iter {
            self.insert(flag);
        }
    }
}

impl IntoIterator for InitFlags {
    type Item = InitFlag;
    type IntoIter = InitFlagsIter;

    fn into_iter(self) -> InitFlagsIter {
        self.iter()
    }
}

/// Lists the named flags, then any unnamed bits in hex: `{Bare | Mkpath, 0x10000}`.
impl fmt::Debug for InitFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("InitFlags {")?;
        let mut sep = "";
        for flag in self.iter() {
            write!(f, "{sep}{flag:?}")?;
            sep = " | ";
        }
        if self.unnamed_bits() != 0 {
            let sep = if sep.is_empty() { "" } else { ", " };
            write!(f, "{sep}{:#x}", self.unnamed_bits())?;
        }
        f.write_str("}")
    }
}

/// The named flags of an [`InitFlags`] set, in ascending bit order
/// ([`InitFlags::iter`]).
///
/// Handwritten
#[derive(Clone, Debug)]
pub struct InitFlagsIter {
    set: InitFlags,
    /// The index into [`InitFlag::ALL`] of the next flag to test.
    next: usize,
}

impl Iterator for InitFlagsIter {
    type Item = InitFlag;

    fn next(&mut self) -> Option<InitFlag> {
        while let Some(&flag) = InitFlag::ALL.get(self.next) {
            self.next += 1;
            if self.set.contains(flag) {
                return Some(flag);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(InitFlag::ALL.len().saturating_sub(self.next)))
    }
}

impl core::iter::FusedIterator for InitFlagsIter {}

/// The permissions `git_repository_init_ext` gives the directories it creates: the
/// `uint32_t` `mode` of `git_repository_init_options`, either one of the
/// `git_repository_init_mode_t` constants or a custom mode.
///
/// Every `u32` is a mode C accepts, so [`from_raw`](Self::from_raw) is total; it
/// gives the named variant for a constant's value, so a [`CustomMode`] never holds
/// one and equal modes compare equal.
///
/// libgit2 creates the repository's directories with the mode's permission bits
/// (`0777` for [`SharedUmask`](Self::SharedUmask)), trimmed by the process umask
/// except where it `chmod`s them: with any other mode, the built-in template's leaf
/// directories (`objects/info`, `objects/pack`, `hooks`, `info`) and an external
/// template's directories get the mode exactly, and a mode with the setgid bit, as
/// both shared ones have, is forced onto the repository directory too. Parent
/// directories that [`InitFlag::Mkdir`] or [`InitFlag::Mkpath`] create never get the
/// setgid bit, and [`InitFlag::Mkpath`]'s ancestors no world-write bit either.
///
/// ```compile_fail,E0423
/// use libgit2::libgit2::repository::repository::{CustomMode, InitMode};
/// // A custom mode comes from `from_raw` only, which names the shared modes.
/// let _ = InitMode::Custom(CustomMode(0o2775));
/// ```
///
/// Wraps: git_repository_init_mode_t
/// Handwritten
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum InitMode {
    /// `0`: directories get `0777` less the umask, and nothing is shared. The
    /// default.
    #[default]
    SharedUmask,
    /// `02775`, git's `--shared=group`: group-writable directories that pass their
    /// group on (`g+sx`); the configuration records `core.sharedrepository = 1` and
    /// `receive.denyNonFastforwards`.
    SharedGroup,
    /// `02777`, git's `--shared=all`: as [`SharedGroup`](Self::SharedGroup), and
    /// world-readable; the configuration records `core.sharedrepository = 2`.
    SharedAll,
    /// Any other value: the directories' permission bits, recorded nowhere in the
    /// configuration.
    Custom(CustomMode),
}

impl InitMode {
    /// The mode whose C value is `raw`: a named variant for a
    /// `git_repository_init_mode_t` constant, [`Custom`](Self::Custom) otherwise.
    pub const fn from_raw(raw: ffi::git_repository_init_mode_t) -> Self {
        match raw {
            ffi::GIT_REPOSITORY_INIT_SHARED_UMASK => Self::SharedUmask,
            ffi::GIT_REPOSITORY_INIT_SHARED_GROUP => Self::SharedGroup,
            ffi::GIT_REPOSITORY_INIT_SHARED_ALL => Self::SharedAll,
            bits => Self::Custom(CustomMode(bits)),
        }
    }

    /// The C value, as the options' `mode` takes it.
    pub const fn raw(self) -> ffi::git_repository_init_mode_t {
        match self {
            Self::SharedUmask => ffi::GIT_REPOSITORY_INIT_SHARED_UMASK,
            Self::SharedGroup => ffi::GIT_REPOSITORY_INIT_SHARED_GROUP,
            Self::SharedAll => ffi::GIT_REPOSITORY_INIT_SHARED_ALL,
            Self::Custom(mode) => mode.0,
        }
    }
}

impl From<ffi::git_repository_init_mode_t> for InitMode {
    fn from(raw: ffi::git_repository_init_mode_t) -> Self {
        Self::from_raw(raw)
    }
}

impl From<InitMode> for ffi::git_repository_init_mode_t {
    fn from(value: InitMode) -> ffi::git_repository_init_mode_t {
        value.raw()
    }
}

/// A custom [`InitMode`]: permission bits other than the three shared modes' values.
/// Build one with [`InitMode::from_raw`].
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CustomMode(ffi::git_repository_init_mode_t);

impl CustomMode {
    /// The permission bits, as the options' `mode` takes them.
    pub const fn bits(self) -> ffi::git_repository_init_mode_t {
        self.0
    }
}

/// Options for `git_repository_init_ext` (`git_repository_init_options`), held by value
/// in Rust storage.
///
/// Built by [`new`](Self::new) (or [`Default`]) with libgit2's defaults: no
/// [`InitFlags`], the [`InitMode::SharedUmask`] mode, no strings, and the default
/// object id type and reference storage. Read it through [`as_ref`](Self::as_ref) and
/// change it through [`as_mut`](Self::as_mut):
///
/// ```
/// use libgit2::libgit2::repository::repository::{InitFlag, InitMode, InitOptions};
///
/// # if cfg!(miri) { return; }
/// let head = c"main";
/// let mut opts = InitOptions::new();
/// let mut m = opts.as_mut();
/// m.set_flags(InitFlag::Bare | InitFlag::Mkpath);
/// m.set_mode(InitMode::SharedGroup);
/// m.set_initial_head(Some(head));
/// assert_eq!(opts.as_ref().initial_head(), Some(c"main"));
/// assert!(opts.as_ref().flags().contains(InitFlag::Bare));
/// ```
///
/// `#[repr(transparent)]` over `git_repository_init_options`, so a pointer to it is a
/// pointer to the C struct, which C reads in place. `'o` is the lifetime of the strings
/// it points at, which its setters borrow, so the options cannot outlive them:
///
/// ```compile_fail,E0597
/// use libgit2::libgit2::repository::repository::InitOptions;
/// use std::ffi::CString;
///
/// let mut opts = InitOptions::new();
/// {
///     let url = CString::new("https://example.com/repo.git").unwrap();
///     opts.as_mut().set_origin_url(Some(&url));
/// }
/// let _ = opts.as_ref().origin_url();
/// ```
///
/// The exclusive handle is invariant in `'o`, so one cannot be shrunk to accept a
/// shorter-lived borrow than its options were built for:
///
/// ```compile_fail
/// use libgit2::libgit2::repository::repository::InitOptionsMut;
///
/// fn shrink<'a, 'o>(m: InitOptionsMut<'a, 'static>) -> InitOptionsMut<'a, 'o> {
///     m
/// }
/// ```
///
/// The shared handle only reads:
///
/// ```compile_fail,E0599
/// use libgit2::libgit2::repository::repository::{InitFlags, InitOptions};
///
/// let opts = InitOptions::new();
/// opts.as_ref().set_flags(InitFlags::empty());
/// ```
///
/// `Copy`, `Send` and `Sync`, as a struct of integers and `&'o CStr`s is: libgit2
/// never writes through the strings, and only the exclusive handle writes the fields.
///
/// `git_repository_init_ext` writes to the options it is given: it ORs internal bits
/// into the flags (see [`InitFlags`]), which a later call with the same options reads.
///
/// Wraps: git_repository_init_options
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct InitOptions<'o>(ffi::git_repository_init_options, PhantomData<&'o CStr>);

/// Shared borrow of an [`InitOptions`]. `Copy`, like `&T`; the getters live here.
///
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct InitOptionsRef<'a, 'o>(CBorrowedPtr<'a, InitOptions<'o>>);

/// Exclusive borrow of an [`InitOptions`]. Reaches the getters with
/// [`as_ref`](Self::as_ref); the setters live here.
///
/// Handwritten
#[repr(transparent)]
pub struct InitOptionsMut<'a, 'o>(
    InitOptionsRef<'a, 'o>,
    // Invariant in the options, and so in `'o`, as `&'a mut` is; see `CCell`.
    PhantomData<&'a mut InitOptions<'o>>,
);

// SAFETY: `InitOptions` is `#[repr(transparent)]` over `git_repository_init_options`
// with a zero-sized marker; both handles are `#[repr(transparent)]` over
// `CBorrowedPtr<'a, InitOptions<'o>>` (`InitOptionsMut` through `InitOptionsRef`) with
// zero-sized markers and no `Drop`, hand out no reference to the options, and
// `InitOptionsRef` has no write operation. `InitOptionsMut` is invariant in the options
// through its `PhantomData<&'a mut InitOptions<'o>>`.
unsafe impl<'o> CCell for InitOptions<'o> {
    type C = ffi::git_repository_init_options;
    type Ref<'a>
        = InitOptionsRef<'a, 'o>
    where
        Self: 'a;
    type Mut<'a>
        = InitOptionsMut<'a, 'o>
    where
        Self: 'a;
}

// SAFETY: the options are integers and five string pointers that only the setters
// write, each NULL or a `&'o CStr`'s pointer: shared, immutable borrows of `CStr`,
// which is `Sync`, so they may be read from any thread. Nothing in the struct has
// thread affinity or interior mutability, and nothing is released on drop.
unsafe impl Send for InitOptions<'_> {}
// SAFETY: as above; `InitOptionsMut` is an exclusive borrow of such a value, as
// `&mut InitOptions`.
unsafe impl Send for InitOptionsMut<'_, '_> {}

// SAFETY: as above, and shared access only reads the fields (every write goes through
// `InitOptionsMut`, which excludes other handles), so concurrent readers race nothing.
unsafe impl Sync for InitOptions<'_> {}
// SAFETY: as above; `InitOptionsRef` is a shared borrow, as `&InitOptions`.
unsafe impl Send for InitOptionsRef<'_, '_> {}
// SAFETY: as above.
unsafe impl Sync for InitOptionsRef<'_, '_> {}
// SAFETY: `&InitOptionsMut` only reaches the getters, through `as_ref`.
unsafe impl Sync for InitOptionsMut<'_, '_> {}

impl<'o> InitOptions<'o> {
    /// The structure version this crate's header declares
    /// (`GIT_REPOSITORY_INIT_OPTIONS_VERSION`), the only one libgit2 accepts.
    pub const VERSION: u32 = ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION;

    /// libgit2's default options, as `git_repository_init_options_init` fills them for
    /// [`VERSION`](Self::VERSION) and `GIT_REPOSITORY_INIT_OPTIONS_INIT` spells them:
    /// the current version, every other field zero or NULL.
    ///
    /// With its own version, `git_repository_init_options_init` only copies the
    /// template: it reaches no global, no allocator and no thread state, so this needs
    /// no initialised library.
    ///
    /// Wraps: git_repository_init_options_init
    pub fn new() -> Self {
        let mut raw = MaybeUninit::<ffi::git_repository_init_options>::uninit();
        // SAFETY: `raw` is writable storage for one `git_repository_init_options`, which
        // `git_repository_init_options_init` overwrites whole with the template for
        // this version; nothing else reads it during the call.
        let rc = unsafe { ffi::git_repository_init_options_init(raw.as_mut_ptr(), Self::VERSION) };
        assert_eq!(
            rc, 0,
            "git_repository_init_options_init refused its own version"
        );
        // SAFETY: a 0 return means the template was copied in: every field is
        // initialised, every pointer NULL.
        Self(unsafe { raw.assume_init() }, PhantomData)
    }

    /// Shared handle to the options — the getters, and the pointer for C calls that
    /// read them.
    pub fn as_ref(&self) -> InitOptionsRef<'_, 'o> {
        // SAFETY: `self` is a live, initialised value borrowed for the handle's lifetime.
        InitOptionsRef(unsafe { CBorrowedPtr::new(NonNull::from(self)) })
    }

    /// Exclusive handle to the options — the setters, and the pointer for C calls that
    /// write them (`git_repository_init_ext`).
    pub fn as_mut(&mut self) -> InitOptionsMut<'_, 'o> {
        // SAFETY: as `as_ref`, from an exclusive borrow, so the pointer carries write
        // provenance and no other handle exists while the result lives.
        let ptr = unsafe { CBorrowedPtr::new(NonNull::from(self)) };
        InitOptionsMut(InitOptionsRef(ptr), PhantomData)
    }
}

impl Default for InitOptions<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for InitOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

impl<'a, 'o> InitOptionsRef<'a, 'o> {
    /// Read-only pointer to the C struct, for FFI calls taking
    /// `const git_repository_init_options *`.
    pub(crate) fn as_ptr(&self) -> *const ffi::git_repository_init_options {
        self.0.as_non_null().as_ptr().cast()
    }

    /// The structure version. [`InitOptions::new`] sets [`InitOptions::VERSION`];
    /// libgit2 refuses the options with `GIT_ERROR_INVALID` if it is 0 or above that.
    ///
    /// Field: git_repository_init_options.version
    pub fn version(&self) -> u32 {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        unsafe { addr_of!((*p).version).read() }
    }

    /// The init flags, every stored bit kept: after a `git_repository_init_ext` call,
    /// the internal bits it ORed in too (see [`InitFlags`]). Empty by default.
    ///
    /// Field: git_repository_init_options.flags
    pub fn flags(&self) -> InitFlags {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        let bits = unsafe { addr_of!((*p).flags).read() };
        InitFlags::from_bits_retain(bits)
    }

    /// The permissions of the directories the repository is created with;
    /// [`InitMode::SharedUmask`] by default.
    ///
    /// Field: git_repository_init_options.mode
    pub fn mode(&self) -> InitMode {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options.
        let raw = unsafe { addr_of!((*p).mode).read() };
        InitMode::from_raw(raw)
    }

    /// The working directory; `None`, the default, means the repository directory's
    /// parent, for a repository path ending in `.git` (one is appended to a path that
    /// does not). Borrowed for `'o`, as it was set.
    ///
    /// Field: git_repository_init_options.workdir_path
    pub fn workdir_path(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).workdir_path).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The content of the new repository's `description` file; `None`, the default,
    /// keeps the template's. Borrowed for `'o`, as it was set.
    ///
    /// Field: git_repository_init_options.description
    pub fn description(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).description).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The template directory; `None`, the default, means the `init.templatedir`
    /// setting or the system's template directory, when an external template is used
    /// at all. Borrowed for `'o`, as it was set.
    ///
    /// Field: git_repository_init_options.template_path
    pub fn template_path(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).template_path).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The branch, or reference, `HEAD` points at; `None`, the default, means the
    /// `init.defaultbranch` setting, or `master`. Borrowed for `'o`, as it was set.
    ///
    /// Field: git_repository_init_options.initial_head
    pub fn initial_head(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).initial_head).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The URL of an `origin` remote to add; `None`, the default, adds none. Borrowed
    /// for `'o`, as it was set.
    ///
    /// Field: git_repository_init_options.origin_url
    pub fn origin_url(&self) -> Option<&'o CStr> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options. Every
        // setter stores NULL or a `&'o CStr`'s pointer, a NUL-terminated string that
        // outlives `'o` and that nothing writes, being shared-borrowed.
        unsafe {
            let s = addr_of!((*p).origin_url).read();
            (!s.is_null()).then(|| CStr::from_ptr(s))
        }
    }

    /// The type of the new repository's object ids; `None`, the default, means
    /// [`OidType::default`]. Every setter stores `0` or a named type, so `None` is
    /// always the zero default.
    ///
    /// Field: git_repository_init_options.oid_type
    pub fn oid_type(&self) -> Option<OidType> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options; the
        // field is a plain `u32`, any value valid.
        let raw = unsafe { addr_of!((*p).oid_type).read() };
        OidType::from_raw(raw)
    }

    /// The storage format of the new repository's references; `None`, the default,
    /// means [`RefdbType::default`]. Every setter stores `0` or a named format, so
    /// `None` is always the zero default.
    ///
    /// Field: git_repository_init_options.refdb_type
    pub fn refdb_type(&self) -> Option<RefdbType> {
        let p = self.as_ptr();
        // SAFETY: a read through the shared handle's pointer to live options; the
        // field is a plain `u32`, any value valid.
        let raw = unsafe { addr_of!((*p).refdb_type).read() };
        RefdbType::from_raw(raw)
    }

    /// A copy of the options, as a value of its own, borrowing the same strings.
    pub fn to_options(&self) -> InitOptions<'o> {
        // SAFETY: a read of the whole live struct through the shared handle's pointer;
        // it is `Copy` plain data, whose string pointers keep their `'o` guarantee in
        // the copy, which is typed with the same `'o`.
        InitOptions(unsafe { self.as_ptr().read() }, PhantomData)
    }
}

impl fmt::Debug for InitOptionsRef<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InitOptions")
            .field("version", &self.version())
            .field("flags", &self.flags())
            .field("mode", &self.mode())
            .field("workdir_path", &self.workdir_path())
            .field("description", &self.description())
            .field("template_path", &self.template_path())
            .field("initial_head", &self.initial_head())
            .field("origin_url", &self.origin_url())
            .field("oid_type", &self.oid_type())
            .field("refdb_type", &self.refdb_type())
            .finish()
    }
}

impl<'o> InitOptionsMut<'_, 'o> {
    /// Writable pointer to the C struct, for the setters and for C calls that write
    /// the options (`git_repository_init_ext`).
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_repository_init_options {
        self.0.0.as_non_null().as_ptr().cast()
    }

    /// Reborrows shared.
    pub fn as_ref(&self) -> InitOptionsRef<'_, 'o> {
        self.0
    }

    /// Reborrows exclusively, keeping this handle frozen while the result lives.
    pub fn as_mut(&mut self) -> InitOptionsMut<'_, 'o> {
        InitOptionsMut(self.0, PhantomData)
    }

    /// Replaces the structure version. Any value is accepted; libgit2 refuses options
    /// whose version is 0 or above [`InitOptions::VERSION`].
    ///
    /// Field: git_repository_init_options.version
    pub fn set_version(&mut self, version: u32) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).version).write(version) }
    }

    /// Replaces the init flags, every bit as given: libgit2 tests the bits
    /// [`InitFlag`] names and its own internal ones (see [`InitFlags`]), and ignores
    /// the rest.
    ///
    /// Field: git_repository_init_options.flags
    pub fn set_flags(&mut self, flags: InitFlags) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).flags).write(flags.bits()) }
    }

    /// Replaces the permissions of the directories the repository is created with.
    ///
    /// Field: git_repository_init_options.mode
    pub fn set_mode(&mut self, mode: InitMode) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, so the old value needs no drop.
        unsafe { addr_of_mut!((*p).mode).write(mode.raw()) }
    }

    /// Sets the working directory; `None` means the repository directory's parent.
    /// libgit2 resolves a relative path against the repository directory and, unless
    /// it is that parent, writes a `.git` file there linking to the repository and
    /// records it as `core.worktree`. Ignored for a bare repository; a non-bare
    /// repository whose path does not end in `.git` needs one. Read during each call,
    /// never kept.
    ///
    /// Field: git_repository_init_options.workdir_path
    pub fn set_workdir_path(&mut self, path: Option<&'o CStr>) {
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data. The value is NULL or a `&'o CStr`'s pointer, as the
        // getter requires.
        unsafe { addr_of_mut!((*p).workdir_path).write(path.map_or(ptr::null(), CStr::as_ptr)) }
    }

    /// Sets the content of the new repository's `description` file; `None` keeps the
    /// template's. libgit2 writes it only into a repository it creates from its
    /// built-in template: a reinitialisation and an external template ignore it. Read
    /// during each call, never kept.
    ///
    /// Field: git_repository_init_options.description
    pub fn set_description(&mut self, description: Option<&'o CStr>) {
        let s = description.map_or(ptr::null(), CStr::as_ptr);
        let p = self.as_mut_ptr();
        // SAFETY: as `set_workdir_path`, for this field.
        unsafe { addr_of_mut!((*p).description).write(s) }
    }

    /// Sets the template directory a new repository is copied from; `None` means the
    /// `init.templatedir` setting or the system's template directory, and only with
    /// [`InitFlag::ExternalTemplate`]. A path makes libgit2 use an external template
    /// whatever the flags; an empty one copies nothing, and a missing directory falls
    /// back to the built-in template. Read during each call, never kept.
    ///
    /// Field: git_repository_init_options.template_path
    pub fn set_template_path(&mut self, path: Option<&'o CStr>) {
        let s = path.map_or(ptr::null(), CStr::as_ptr);
        let p = self.as_mut_ptr();
        // SAFETY: as `set_workdir_path`, for this field.
        unsafe { addr_of_mut!((*p).template_path).write(s) }
    }

    /// Sets the branch `HEAD` points at: a name below `refs/heads/`, or a full
    /// reference name starting with `refs/`. `None` means the `init.defaultbranch`
    /// setting, or `master`, and leaves a reinitialised repository's `HEAD` alone.
    /// Read during each call, never kept.
    ///
    /// Field: git_repository_init_options.initial_head
    pub fn set_initial_head(&mut self, head: Option<&'o CStr>) {
        let s = head.map_or(ptr::null(), CStr::as_ptr);
        let p = self.as_mut_ptr();
        // SAFETY: as `set_workdir_path`, for this field.
        unsafe { addr_of_mut!((*p).initial_head).write(s) }
    }

    /// Sets the URL of an `origin` remote libgit2 adds once the repository is
    /// initialised; `None` adds none. Read during each call, never kept.
    ///
    /// Field: git_repository_init_options.origin_url
    pub fn set_origin_url(&mut self, url: Option<&'o CStr>) {
        let s = url.map_or(ptr::null(), CStr::as_ptr);
        let p = self.as_mut_ptr();
        // SAFETY: as `set_workdir_path`, for this field.
        unsafe { addr_of_mut!((*p).origin_url).write(s) }
    }

    /// Sets the type of the new repository's object ids; `None` stores `0`, which
    /// libgit2 reads as [`OidType::default`]. A type other than the default is
    /// recorded as `extensions.objectformat`.
    ///
    /// Field: git_repository_init_options.oid_type
    pub fn set_oid_type(&mut self, oid_type: Option<OidType>) {
        let raw = oid_type.map_or(0, OidType::raw);
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, and `raw` is 0 or a type libgit2 names.
        unsafe { addr_of_mut!((*p).oid_type).write(raw) }
    }

    /// Sets the storage format of the new repository's references; `None` stores `0`,
    /// which libgit2 reads as [`RefdbType::default`]. A format other than the default
    /// is recorded as `extensions.refstorage`.
    ///
    /// Field: git_repository_init_options.refdb_type
    pub fn set_refdb_type(&mut self, refdb_type: Option<RefdbType>) {
        let raw = refdb_type.map_or(0, RefdbType::raw);
        let p = self.as_mut_ptr();
        // SAFETY: a write through the exclusive handle's pointer to live options; the
        // field is plain data, and `raw` is 0 or a format libgit2 names.
        unsafe { addr_of_mut!((*p).refdb_type).write(raw) }
    }
}

impl fmt::Debug for InitOptionsMut<'_, '_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

// An `InitOptions` is a `git_repository_init_options`, so C reads a value Rust holds in
// place.
const _: () = assert!(
    size_of::<InitOptions<'static>>() == size_of::<ffi::git_repository_init_options>()
        && align_of::<InitOptions<'static>>() == align_of::<ffi::git_repository_init_options>()
);

/// One file or directory of a repository's layout, as `git_repository_item_path`
/// resolves it: against the git directory ([`GitDir`](Self::GitDir), the `.git`
/// directory or a bare repository itself), the working directory
/// ([`WorkDir`](Self::WorkDir)), or the common directory a worktree shares with its
/// main repository ([`CommonDir`](Self::CommonDir)), falling back to the git directory
/// for a repository without one. A directory resolves with a trailing `/`; an item
/// whose path cannot exist (the working directory of a bare repository) resolves to
/// `GIT_ENOTFOUND`.
///
/// [`from_raw`](Self::from_raw) gives `None` for `GIT_REPOSITORY_ITEM__LAST` and every
/// value after it, which libgit2 would read past its item table with.
///
/// Wraps: git_repository_item_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum Item {
    /// The git directory, `<gitdir>/`.
    GitDir = ffi::GIT_REPOSITORY_ITEM_GITDIR,
    /// The working directory, `<workdir>/`; not found in a bare repository.
    WorkDir = ffi::GIT_REPOSITORY_ITEM_WORKDIR,
    /// The common directory, `<commondir>/`: the main repository's git directory for
    /// a worktree, the git directory otherwise.
    CommonDir = ffi::GIT_REPOSITORY_ITEM_COMMONDIR,
    /// The index file, `<gitdir>/index`.
    Index = ffi::GIT_REPOSITORY_ITEM_INDEX,
    /// The object database, `<commondir>/objects/`.
    Objects = ffi::GIT_REPOSITORY_ITEM_OBJECTS,
    /// The loose references, `<commondir>/refs/`.
    Refs = ffi::GIT_REPOSITORY_ITEM_REFS,
    /// The packed references file, `<commondir>/packed-refs`.
    PackedRefs = ffi::GIT_REPOSITORY_ITEM_PACKED_REFS,
    /// The legacy remotes directory, `<commondir>/remotes/`.
    Remotes = ffi::GIT_REPOSITORY_ITEM_REMOTES,
    /// The repository's configuration file, `<commondir>/config`.
    Config = ffi::GIT_REPOSITORY_ITEM_CONFIG,
    /// The info directory, `<commondir>/info/`.
    Info = ffi::GIT_REPOSITORY_ITEM_INFO,
    /// The hooks directory, `<commondir>/hooks/`.
    Hooks = ffi::GIT_REPOSITORY_ITEM_HOOKS,
    /// The reflogs, `<commondir>/logs/`.
    Logs = ffi::GIT_REPOSITORY_ITEM_LOGS,
    /// The submodules' git directories, `<gitdir>/modules/`.
    Modules = ffi::GIT_REPOSITORY_ITEM_MODULES,
    /// The worktrees' administrative directories, `<commondir>/worktrees/`.
    Worktrees = ffi::GIT_REPOSITORY_ITEM_WORKTREES,
    /// The per-worktree configuration file, `<gitdir>/config.worktree`.
    WorktreeConfig = ffi::GIT_REPOSITORY_ITEM_WORKTREE_CONFIG,
}

impl Item {
    /// Every variant, in C value order: `ALL[n]` is the item whose value is `n`.
    pub const ALL: [Item; 15] = [
        Self::GitDir,
        Self::WorkDir,
        Self::CommonDir,
        Self::Index,
        Self::Objects,
        Self::Refs,
        Self::PackedRefs,
        Self::Remotes,
        Self::Config,
        Self::Info,
        Self::Hooks,
        Self::Logs,
        Self::Modules,
        Self::Worktrees,
        Self::WorktreeConfig,
    ];

    /// The variant whose C value is `raw`; `None` for `GIT_REPOSITORY_ITEM__LAST` and
    /// every value after it.
    pub const fn from_raw(raw: ffi::git_repository_item_t) -> Option<Self> {
        // `ALL` is indexed by value and ends at `__LAST`, the bound of libgit2's table.
        const {
            assert!(Item::ALL.len() == ffi::GIT_REPOSITORY_ITEM__LAST as usize);
            let mut i = 0;
            while i < Item::ALL.len() {
                assert!(Item::ALL[i] as usize == i);
                i += 1;
            }
        }
        if raw < ffi::GIT_REPOSITORY_ITEM__LAST {
            Some(Self::ALL[raw as usize])
        } else {
            None
        }
    }

    /// The C value, as `git_repository_item_path` takes it; always below
    /// `GIT_REPOSITORY_ITEM__LAST`.
    pub const fn raw(self) -> ffi::git_repository_item_t {
        self as ffi::git_repository_item_t
    }
}

impl TryFrom<ffi::git_repository_item_t> for Item {
    type Error = UnnamedItem;

    fn try_from(raw: ffi::git_repository_item_t) -> Result<Self, UnnamedItem> {
        Self::from_raw(raw).ok_or(UnnamedItem(raw))
    }
}

impl From<Item> for ffi::git_repository_item_t {
    fn from(value: Item) -> ffi::git_repository_item_t {
        value.raw()
    }
}

/// A repository item value that names no item: `GIT_REPOSITORY_ITEM__LAST` or a value
/// after it. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedItem(ffi::git_repository_item_t);

impl UnnamedItem {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_repository_item_t {
        self.0
    }
}

impl fmt::Display for UnnamedItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 repository item {} is not named by git2/repository.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedItem {}

/// One option of `git_repository_open_ext`'s search for a repository, a single bit of
/// an [`OpenFlags`] set.
///
/// Without any, the search starts at the given path, tries `<path>/.git` and then
/// `<path>` itself, and walks up through the parent directories until a ceiling
/// directory, staying on the starting file system.
///
/// [`from_raw`](Self::from_raw) gives `None` for any value without a variant: `0`, a
/// combination of bits, or a bit this version of libgit2's header does not name.
///
/// Wraps: git_repository_open_flag_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum OpenFlag {
    /// Only look at the starting path (and its `.git`); do not walk up through its
    /// parent directories.
    NoSearch = ffi::GIT_REPOSITORY_OPEN_NO_SEARCH,
    /// Keep walking up across file system boundaries (a change of `st_dev`).
    CrossFs = ffi::GIT_REPOSITORY_OPEN_CROSS_FS,
    /// Open the repository as bare whatever its `core.bare`, deferring the loading of
    /// its configuration; unlike `git_repository_open_bare`, a gitlink is followed.
    /// The search tries the starting path itself and not its `.git`; the parents it
    /// walks up to are still tried with `.git` first.
    Bare = ffi::GIT_REPOSITORY_OPEN_BARE,
    /// Never append `/.git`: only a git directory itself is a repository.
    NoDotGit = ffi::GIT_REPOSITORY_OPEN_NO_DOTGIT,
    /// Respect the git command-line tools' environment variables: `GIT_DIR` (for a
    /// NULL path, which it then opens as a git directory without searching),
    /// `GIT_CEILING_DIRECTORIES` (for NULL ceiling directories),
    /// `GIT_DISCOVERY_ACROSS_FILESYSTEM` (a boolean, adding [`CrossFs`](Self::CrossFs);
    /// any other value fails the open), `GIT_COMMON_DIR`, `GIT_WORK_TREE`,
    /// `GIT_INDEX_FILE`, `GIT_NAMESPACE`, `GIT_OBJECT_DIRECTORY`,
    /// `GIT_ALTERNATE_OBJECT_DIRECTORIES` and the `GIT_CONFIG_*` paths. Unlike what
    /// libgit2's header says, the other flags and given ceiling directories still
    /// apply.
    FromEnv = ffi::GIT_REPOSITORY_OPEN_FROM_ENV,
}

impl OpenFlag {
    /// Every variant, in ascending bit order.
    pub const ALL: [OpenFlag; 5] = [
        Self::NoSearch,
        Self::CrossFs,
        Self::Bare,
        Self::NoDotGit,
        Self::FromEnv,
    ];

    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_repository_open_flag_t) -> Option<Self> {
        let mut i = 0;
        while i < Self::ALL.len() {
            if Self::ALL[i] as ffi::git_repository_open_flag_t == raw {
                return Some(Self::ALL[i]);
            }
            i += 1;
        }
        None
    }

    /// The C value, a single bit.
    pub const fn raw(self) -> ffi::git_repository_open_flag_t {
        self as ffi::git_repository_open_flag_t
    }

    /// The flag's bit in the `unsigned int` `git_repository_open_ext` takes.
    const fn bit(self) -> c_uint {
        self as c_uint
    }
}

impl TryFrom<ffi::git_repository_open_flag_t> for OpenFlag {
    type Error = UnnamedOpenFlag;

    fn try_from(raw: ffi::git_repository_open_flag_t) -> Result<Self, UnnamedOpenFlag> {
        Self::from_raw(raw).ok_or(UnnamedOpenFlag(raw))
    }
}

impl From<OpenFlag> for ffi::git_repository_open_flag_t {
    fn from(value: OpenFlag) -> ffi::git_repository_open_flag_t {
        value.raw()
    }
}

/// An open flag value that this version of libgit2's header does not name: `0`, a
/// combination of flags, or a later header's bit. [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedOpenFlag(ffi::git_repository_open_flag_t);

impl UnnamedOpenFlag {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_repository_open_flag_t {
        self.0
    }
}

impl fmt::Display for UnnamedOpenFlag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 repository open flag {} is not named by git2/repository.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedOpenFlag {}

/// A set of [`OpenFlag`]s: the `unsigned int` combination of `GIT_REPOSITORY_OPEN_*`
/// bits that `git_repository_open_ext` takes. The empty set is the default search.
///
/// It keeps every bit it is given, named or not, so [`bits`](Self::bits) passes on
/// what it was built from; libgit2 ignores the bits no [`OpenFlag`] names.
/// [`iter`](Self::iter) and [`contains`](Self::contains) see the named flags, and
/// [`unnamed_bits`](Self::unnamed_bits) the rest.
///
/// Wraps: git_repository_open_flag_t
/// Handwritten
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct OpenFlags(c_uint);

impl OpenFlags {
    /// The bits of every [`OpenFlag`].
    const NAMED: c_uint = {
        let mut bits = 0;
        let mut i = 0;
        while i < OpenFlag::ALL.len() {
            bits |= OpenFlag::ALL[i].bit();
            i += 1;
        }
        bits
    };

    /// The set with no flag: the default search.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The set of every bit of `bits`; bits that no [`OpenFlag`] names are kept.
    pub const fn from_bits_retain(bits: c_uint) -> Self {
        Self(bits)
    }

    /// The bits of the set, as `git_repository_open_ext` takes them.
    pub const fn bits(self) -> c_uint {
        self.0
    }

    /// The bits of the set that no [`OpenFlag`] names.
    pub const fn unnamed_bits(self) -> c_uint {
        self.0 & !Self::NAMED
    }

    /// Whether the set has no bit, named or not.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether `flag` is in the set.
    pub const fn contains(self, flag: OpenFlag) -> bool {
        self.0 & flag.bit() != 0
    }

    /// Adds `flag` to the set.
    pub fn insert(&mut self, flag: OpenFlag) {
        self.0 |= flag.bit();
    }

    /// Removes `flag` from the set.
    pub fn remove(&mut self, flag: OpenFlag) {
        self.0 &= !flag.bit();
    }

    /// The named flags in the set, in ascending bit order.
    pub fn iter(self) -> OpenFlagsIter {
        OpenFlagsIter { set: self, next: 0 }
    }
}

impl From<OpenFlag> for OpenFlags {
    fn from(flag: OpenFlag) -> Self {
        Self(flag.bit())
    }
}

impl BitOr for OpenFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<OpenFlag> for OpenFlags {
    type Output = Self;

    fn bitor(self, rhs: OpenFlag) -> Self {
        self | Self::from(rhs)
    }
}

impl BitOr for OpenFlag {
    type Output = OpenFlags;

    fn bitor(self, rhs: Self) -> OpenFlags {
        OpenFlags::from(self) | rhs
    }
}

impl BitOrAssign for OpenFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl BitOrAssign<OpenFlag> for OpenFlags {
    fn bitor_assign(&mut self, rhs: OpenFlag) {
        self.insert(rhs);
    }
}

impl FromIterator<OpenFlag> for OpenFlags {
    fn from_iter<I: IntoIterator<Item = OpenFlag>>(iter: I) -> Self {
        let mut set = Self::empty();
        set.extend(iter);
        set
    }
}

impl Extend<OpenFlag> for OpenFlags {
    fn extend<I: IntoIterator<Item = OpenFlag>>(&mut self, iter: I) {
        for flag in iter {
            self.insert(flag);
        }
    }
}

impl IntoIterator for OpenFlags {
    type Item = OpenFlag;
    type IntoIter = OpenFlagsIter;

    fn into_iter(self) -> OpenFlagsIter {
        self.iter()
    }
}

/// Lists the named flags, then any unnamed bits in hex: `{NoSearch | Bare, 0x20}`.
impl fmt::Debug for OpenFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OpenFlags {")?;
        let mut sep = "";
        for flag in self.iter() {
            write!(f, "{sep}{flag:?}")?;
            sep = " | ";
        }
        if self.unnamed_bits() != 0 {
            let sep = if sep.is_empty() { "" } else { ", " };
            write!(f, "{sep}{:#x}", self.unnamed_bits())?;
        }
        f.write_str("}")
    }
}

/// The named flags of an [`OpenFlags`] set, in ascending bit order
/// ([`OpenFlags::iter`]).
///
/// Handwritten
#[derive(Clone, Debug)]
pub struct OpenFlagsIter {
    set: OpenFlags,
    /// The index into [`OpenFlag::ALL`] of the next flag to test.
    next: usize,
}

impl Iterator for OpenFlagsIter {
    type Item = OpenFlag;

    fn next(&mut self) -> Option<OpenFlag> {
        while let Some(&flag) = OpenFlag::ALL.get(self.next) {
            self.next += 1;
            if self.set.contains(flag) {
                return Some(flag);
            }
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, Some(OpenFlag::ALL.len().saturating_sub(self.next)))
    }
}

impl core::iter::FusedIterator for OpenFlagsIter {}

/// The operation a repository is in the middle of: a merge, revert, cherry-pick,
/// bisect, rebase or mailbox application that has stopped, typically on a conflict,
/// and left its marker files in the git directory for a later step to resume.
///
/// `git_repository_state` probes the markers in a fixed order and reports the first it
/// finds, so a repository carrying several (an interrupted rebase whose last step
/// stopped on a merge) reports the earliest-probed one. The order is
/// [`RebaseInteractive`](Self::RebaseInteractive), [`RebaseMerge`](Self::RebaseMerge),
/// [`Rebase`](Self::Rebase), [`ApplyMailbox`](Self::ApplyMailbox),
/// [`ApplyMailboxOrRebase`](Self::ApplyMailboxOrRebase), [`Merge`](Self::Merge), the
/// revert states, the cherry-pick states, then [`Bisect`](Self::Bisect); a sequence
/// state refines its single-commit one. `git_repository_state_cleanup` removes every
/// marker, the rebase directories and the sequencer included, which leaves the
/// repository [`Clean`](Self::Clean).
///
/// [`from_raw`](Self::from_raw) gives `None` for every value the header does not name.
///
/// Wraps: git_repository_state_t
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u32)]
#[non_exhaustive]
pub enum RepositoryState {
    /// No operation is in progress (`GIT_REPOSITORY_STATE_NONE`).
    Clean = ffi::GIT_REPOSITORY_STATE_NONE,
    /// A merge has stopped: `MERGE_HEAD` exists.
    Merge = ffi::GIT_REPOSITORY_STATE_MERGE,
    /// A revert of one commit has stopped: the `REVERT_HEAD` reference resolves.
    Revert = ffi::GIT_REPOSITORY_STATE_REVERT,
    /// A revert of a series of commits has stopped: `REVERT_HEAD` resolves and
    /// `sequencer/todo` exists.
    RevertSequence = ffi::GIT_REPOSITORY_STATE_REVERT_SEQUENCE,
    /// A cherry-pick of one commit has stopped: the `CHERRY_PICK_HEAD` reference
    /// resolves.
    CherryPick = ffi::GIT_REPOSITORY_STATE_CHERRYPICK,
    /// A cherry-pick of a series of commits has stopped: `CHERRY_PICK_HEAD` resolves
    /// and `sequencer/todo` exists.
    CherryPickSequence = ffi::GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE,
    /// A bisection is in progress: `BISECT_LOG` exists.
    Bisect = ffi::GIT_REPOSITORY_STATE_BISECT,
    /// A patch-based rebase has stopped: `rebase-apply/rebasing` exists.
    Rebase = ffi::GIT_REPOSITORY_STATE_REBASE,
    /// An interactive rebase has stopped: `rebase-merge/interactive` exists.
    RebaseInteractive = ffi::GIT_REPOSITORY_STATE_REBASE_INTERACTIVE,
    /// A merge-based rebase has stopped: the `rebase-merge/` directory exists.
    RebaseMerge = ffi::GIT_REPOSITORY_STATE_REBASE_MERGE,
    /// A mailbox application (`git am`) has stopped: `rebase-apply/applying` exists.
    ApplyMailbox = ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX,
    /// A mailbox application or a patch-based rebase has stopped: the `rebase-apply/`
    /// directory exists, without the file that tells the two apart.
    ApplyMailboxOrRebase = ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE,
}

impl RepositoryState {
    /// The variant whose C value is `raw`; `None` for a value this version of
    /// libgit2's header does not name.
    pub const fn from_raw(raw: ffi::git_repository_state_t) -> Option<Self> {
        match raw {
            ffi::GIT_REPOSITORY_STATE_NONE => Some(Self::Clean),
            ffi::GIT_REPOSITORY_STATE_MERGE => Some(Self::Merge),
            ffi::GIT_REPOSITORY_STATE_REVERT => Some(Self::Revert),
            ffi::GIT_REPOSITORY_STATE_REVERT_SEQUENCE => Some(Self::RevertSequence),
            ffi::GIT_REPOSITORY_STATE_CHERRYPICK => Some(Self::CherryPick),
            ffi::GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE => Some(Self::CherryPickSequence),
            ffi::GIT_REPOSITORY_STATE_BISECT => Some(Self::Bisect),
            ffi::GIT_REPOSITORY_STATE_REBASE => Some(Self::Rebase),
            ffi::GIT_REPOSITORY_STATE_REBASE_INTERACTIVE => Some(Self::RebaseInteractive),
            ffi::GIT_REPOSITORY_STATE_REBASE_MERGE => Some(Self::RebaseMerge),
            ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX => Some(Self::ApplyMailbox),
            ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE => Some(Self::ApplyMailboxOrRebase),
            _ => None,
        }
    }

    /// The C value, as libgit2 reports it.
    pub const fn raw(self) -> ffi::git_repository_state_t {
        self as ffi::git_repository_state_t
    }

    /// Whether no operation is in progress: the state is [`Clean`](Self::Clean).
    pub const fn is_clean(self) -> bool {
        matches!(self, Self::Clean)
    }
}

impl TryFrom<ffi::git_repository_state_t> for RepositoryState {
    type Error = UnnamedRepositoryState;

    fn try_from(raw: ffi::git_repository_state_t) -> Result<Self, UnnamedRepositoryState> {
        Self::from_raw(raw).ok_or(UnnamedRepositoryState(raw))
    }
}

impl From<RepositoryState> for ffi::git_repository_state_t {
    fn from(value: RepositoryState) -> ffi::git_repository_state_t {
        value.raw()
    }
}

/// A repository state that this version of libgit2's header does not name.
/// [`raw`](Self::raw) has it.
///
/// Handwritten
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnnamedRepositoryState(ffi::git_repository_state_t);

impl UnnamedRepositoryState {
    /// The value, as it was converted.
    pub const fn raw(self) -> ffi::git_repository_state_t {
        self.0
    }
}

impl fmt::Display for UnnamedRepositoryState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "libgit2 repository state {} is not named by git2/repository.h",
            self.0
        )
    }
}

impl std::error::Error for UnnamedRepositoryState {}

define_ctype!(
    /// A repository (`git_repository`): an opaque C object holding a repository's
    /// paths, its lazily loaded object database, reference database, config and
    /// index, and the caches libgit2 keeps over them.
    ///
    /// libgit2 allocates every repository and releases it with `git_repository_free`;
    /// it keeps no count, so a repository has one owner, a [`RepositoryBox`], built by
    /// [`new`](Self::new), [`with_oid_type`](Self::with_oid_type), [`open`](Self::open),
    /// [`open_bare`](Self::open_bare) or [`init`](Self::init), and read and written
    /// through the box's [`as_ref`](CBox::as_ref) / [`as_mut`](CBox::as_mut) handles.
    /// There is no copy: libgit2 has none, so the box is not `Clone`.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::new().unwrap();
    /// let _copy = repo.clone();
    /// ```
    ///
    /// The objects a repository spawns (objects, references, iterators...) keep a bare
    /// pointer to it and read through it until they are freed, so their wrappers
    /// borrow its handles; a handle cannot outlive the box:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::new().unwrap();
    /// let handle = repo.as_ref();
    /// drop(repo);
    /// let _ = handle;
    /// ```
    ///
    /// The odb, refdb, config and index a repository loads are counted objects it
    /// shares with whoever asks for them (`git_repository_odb`...), on any thread, and
    /// many of its routines fill or replace them, and its other caches, through a
    /// shared pointer. So neither a repository nor its handles are `Send` or `Sync`:
    /// every handle stays on the thread that owns the box, and a routine that updates
    /// those caches is sound through the shared [`RepositoryRef`], as a `Cell` is.
    ///
    /// ```compile_fail,E0277
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// fn send<T: Send>(_: T) {}
    /// send(Repository::new().unwrap());
    /// ```
    ///
    /// Wraps: git_repository
    Repository, RepositoryRef, RepositoryMut, ffi::git_repository
);

/// Releases a repository through `git_repository_free`: the counted references it
/// holds to its odb, refdb, config and index, its caches and paths, then the struct.
/// Also builds one through `git_repository_new`, so [`CBox::new`] makes the
/// repository [`Repository::new`] makes, reporting a failure as `None`.
///
/// No repository reaches C before libgit2 is initialised: each constructor pins the
/// runtime first, so the free is ordered after the initialisation that settled the
/// allocator it frees into.
///
/// Wraps: git_repository_free
/// Wraps: git_repository_new
#[derive(Clone, Copy, Debug, Default)]
pub struct RepositoryFree;

impl_cdrop!(RepositoryFree, Repository, ffi::git_repository_free);

// SAFETY: `git_repository_new` writes a fresh, fully initialised repository that the
// caller owns alone and `git_repository_free` releases, or leaves the slot NULL when
// it fails (`git_repository_new_ext` stores `repository_alloc`'s result before
// checking it). The runtime is pinned before the call.
unsafe impl CNew<Repository> for RepositoryFree {
    fn c_new(&self) -> Option<NonNull<Repository>> {
        if !runtime_pinned() {
            return None;
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; the runtime is pinned, so a failure is
        // reported on live thread-local state.
        let rc = unsafe { ffi::git_repository_new(&mut out) };
        if rc != 0 {
            return None;
        }
        NonNull::new(out.cast())
    }
}

/// An owned [`Repository`], freed by `git_repository_free` on drop.
pub type RepositoryBox = CBox<Repository, RepositoryFree>;

/// A repository constructor's failure: the code it returned, and the error it left as
/// the thread's last libgit2 error, copied out.
///
/// The code tells the failures apart: [`ErrorCode::NotFound`] when there is no
/// repository at the path, [`ErrorCode::Owner`] when it belongs to another user and
/// is not a `safe.directory`, [`ErrorCode::Exists`] when a directory cannot be made
/// over a file, [`ErrorCode::Generic`] for the rest.
///
/// Handwritten
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct RepositoryError {
    code: c_int,
    error: ErrorBuf,
}

impl RepositoryError {
    /// The failure a constructor reported by returning `rc`, with the thread's last
    /// error. A positive `rc` is a failure too (see [`Repository::init`]), reported as
    /// `GIT_ERROR`.
    fn from_libgit2(rc: c_int) -> Self {
        Self {
            code: if rc < 0 { rc } else { ErrorCode::Generic.raw() },
            error: last(),
        }
    }

    /// The returned code; `None` for one the header does not name
    /// ([`raw_code`](Self::raw_code) has it).
    pub fn code(&self) -> Option<ErrorCode> {
        ErrorCode::from_raw(self.code)
    }

    /// The returned code, always negative.
    pub fn raw_code(&self) -> c_int {
        self.code
    }

    /// The error's class; `None` for a class the header does not name
    /// ([`raw_class`](Self::raw_class) has it).
    pub fn class(&self) -> Option<ErrorClass> {
        self.error.class()
    }

    /// The error's class as libgit2 stored it.
    pub fn raw_class(&self) -> c_int {
        self.error.raw_class()
    }

    /// The error message.
    pub fn message(&self) -> &CStr {
        self.error.message()
    }

    /// The error, without the code.
    pub fn error(&self) -> &ErrorBuf {
        &self.error
    }
}

/// The error, without the code.
impl From<RepositoryError> for ErrorBuf {
    fn from(error: RepositoryError) -> Self {
        error.error
    }
}

impl fmt::Debug for RepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("RepositoryError");
        match self.code() {
            Some(code) => s.field("code", &code),
            None => s.field("code", &self.code),
        };
        s.field("error", &self.error).finish()
    }
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for RepositoryError {}

/// The repository a constructor returned `rc` for, adopted from its out slot.
///
/// `git_repository_init` can fail after it opened the repository, leaving it in the
/// slot (and then may return a positive code, see [`Repository::init`]); it is
/// released here, after the error is copied out. A slot left NULL on success would be
/// a libgit2 bug, reported as the thread's error.
fn constructed(rc: c_int, repo: Option<RepositoryBox>) -> Result<RepositoryBox, RepositoryError> {
    match repo {
        Some(repo) if rc == 0 => Ok(repo),
        repo => {
            let error = RepositoryError::from_libgit2(rc);
            drop(repo);
            Err(error)
        }
    }
}

/// The failure reported when libgit2 cannot be initialised, without calling it.
fn unpinned() -> RepositoryError {
    RepositoryError {
        code: ErrorCode::Generic.raw(),
        error: last(),
    }
}

impl Repository {
    /// A repository with no paths, backends or config (`is_bare`), for wiring custom
    /// backends into; its objects are SHA-1 ids. Fails only when libgit2 runs out of
    /// memory, or cannot be initialised.
    ///
    /// Wraps: git_repository_new
    pub fn new() -> Result<RepositoryBox, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; the runtime is pinned, so a failure is
        // reported on live thread-local state.
        let rc = unsafe { ffi::git_repository_new(&mut out) };
        // SAFETY: `git_repository_new` leaves `out` NULL on failure, or writes a fresh
        // repository the caller owns alone.
        constructed(rc, unsafe { RepositoryBox::from_c(out) })
    }

    /// As [`new`](Self::new), for objects of type `oid_type`.
    ///
    /// libgit2 asserts the type is one it names, which [`OidType`] guarantees.
    ///
    /// Wraps: git_repository_new_ext
    pub fn with_oid_type(oid_type: OidType) -> Result<RepositoryBox, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        let mut opts = NewOptions::new();
        let mut m = opts.as_mut();
        m.set_oid_type(Some(oid_type));
        let opts = m.as_mut_ptr();
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot and `opts` live options of the current
        // version naming a valid type, read for the call only. The runtime is pinned.
        let rc = unsafe { ffi::git_repository_new_ext(&mut out, opts) };
        // SAFETY: `git_repository_new_ext` leaves `out` NULL on failure, or writes a
        // fresh repository the caller owns alone.
        constructed(rc, unsafe { RepositoryBox::from_c(out) })
    }

    /// Opens the repository at `path`: a repository directory (`.git`, or a bare
    /// one), or a working directory with one in `.git`. Parent directories are not
    /// searched.
    ///
    /// Fails with [`ErrorCode::NotFound`] when `path` does not resolve or holds no
    /// repository, with [`ErrorCode::Owner`] when the repository belongs to another
    /// user and is not a `safe.directory`, and otherwise with libgit2's error (an
    /// unsupported format version or extension, say).
    ///
    /// Wraps: git_repository_open
    pub fn open(path: &CStr) -> Result<RepositoryBox, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot and `path` a NUL-terminated string, read for
        // the call only. The runtime is pinned.
        let rc = unsafe { ffi::git_repository_open(&mut out, path.as_ptr()) };
        // SAFETY: `git_repository_open` NULLs `out` first and writes a fresh repository
        // the caller owns alone only on success, freeing its own on failure.
        constructed(rc, unsafe { RepositoryBox::from_c(out) })
    }

    /// Opens the repository directory `path` as a bare repository, without reading
    /// its config for a working directory, its grafts or the environment, and without
    /// checking who owns it: a fast open for hosting.
    ///
    /// Fails with [`ErrorCode::NotFound`] when `path` does not resolve or is not a
    /// repository directory (a working directory is not), and otherwise with
    /// libgit2's error.
    ///
    /// libgit2 leaks the half-built repository when it fails after finding one (an
    /// unsupported format version or extension, an allocation failure).
    ///
    /// Wraps: git_repository_open_bare
    pub fn open_bare(path: &CStr) -> Result<RepositoryBox, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot and `path` a NUL-terminated string, read for
        // the call only. The runtime is pinned.
        let rc = unsafe { ffi::git_repository_open_bare(&mut out, path.as_ptr()) };
        // SAFETY: `git_repository_open_bare` writes a fresh repository the caller owns
        // alone only on success, and leaves the NULL slot alone otherwise.
        constructed(rc, unsafe { RepositoryBox::from_c(out) })
    }

    /// Creates a repository at `path`, its missing parent directories included, and
    /// opens it: `path` is the repository directory when `bare`, otherwise the working
    /// directory, the repository going in `path/.git`. An existing repository is
    /// reinitialised, its config refreshed and its references kept.
    ///
    /// Fails with libgit2's error; with [`ErrorCode::Exists`] when a directory on the
    /// way is a file.
    ///
    /// libgit2 can fail after opening the repository (when its reference database
    /// cannot be set up), leaving it in the out slot, which is released here. One such
    /// failure, a reftable repository's `reftable` directory that cannot be made,
    /// returns 1 rather than a negative code: it is reported as an error all the same,
    /// with `GIT_ERROR`, where C callers that test for a negative code take it for
    /// success.
    ///
    /// Wraps: git_repository_init
    pub fn init(path: &CStr, bare: bool) -> Result<RepositoryBox, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, NULL so a failure before the inner open
        // leaves it NULL, and `path` a NUL-terminated string, read for the call only.
        // The runtime is pinned.
        let rc = unsafe { ffi::git_repository_init(&mut out, path.as_ptr(), bare.into()) };
        // SAFETY: `git_repository_init` leaves `out` NULL, or writes the fresh
        // repository it opened, owned by the caller alone, whatever it returns.
        constructed(rc, unsafe { RepositoryBox::from_c(out) })
    }
}

#[cfg(test)]
mod unit_tests {
    mod init_flags_and_modes {
        use super::super::*;

        #[test]
        fn flags_debug_lists_named_flags_then_unnamed_bits() {
            assert_eq!(format!("{:?}", InitFlags::empty()), "InitFlags {}");
            assert_eq!(
                format!("{:?}", InitFlag::Bare | InitFlag::Mkpath),
                "InitFlags {Bare | Mkpath}"
            );
            assert_eq!(
                format!("{:?}", InitFlags::from_bits_retain(0x10004)),
                "InitFlags {0x10004}"
            );
            assert_eq!(
                format!("{:?}", InitFlags::from_bits_retain(0x70011)),
                "InitFlags {Bare | Mkpath, 0x70000}"
            );
        }

        #[test]
        fn flags_build_from_operators_and_iterators() {
            let mut set: InitFlags = [InitFlag::Mkdir, InitFlag::Bare].into_iter().collect();
            assert_eq!(set.bits(), 0x9);
            set |= InitFlag::NoReinit;
            set |= InitFlags::from(InitFlag::RelativeGitlink);
            assert_eq!(set, InitFlags::from_bits_retain(0x4b));
            assert_eq!(
                set.into_iter().collect::<Vec<_>>(),
                [
                    InitFlag::Bare,
                    InitFlag::NoReinit,
                    InitFlag::Mkdir,
                    InitFlag::RelativeGitlink
                ]
            );
            set.remove(InitFlag::Bare);
            set.remove(InitFlag::Bare);
            assert!(!set.contains(InitFlag::Bare));
            set.extend([InitFlag::ExternalTemplate]);
            assert_eq!(set.bits(), 0x6a);
            assert_eq!(InitFlags::default(), InitFlags::empty());
            assert!(InitFlags::default().is_empty());
        }

        #[test]
        fn flags_keep_or_drop_unnamed_bits() {
            // Bit 2 sits between `NoReinit` and `Mkdir`; 16..18 are init_ext's own.
            let raw = 0x7_0000 | 0x4 | InitFlag::Bare.raw();
            let kept = InitFlags::from_bits_retain(raw);
            assert_eq!(kept.bits(), raw);
            assert_eq!(kept.unnamed_bits(), 0x7_0004);
            assert!(!kept.is_empty());
            let truncated = InitFlags::from_bits_truncate(raw);
            assert_eq!(truncated, InitFlag::Bare.into());
            assert_eq!(truncated.unnamed_bits(), 0);
            assert_eq!(InitFlags::from_bits_truncate(u32::MAX).bits(), 0x7b);
            assert!(InitFlags::from_bits_retain(0x4).iter().next().is_none());
        }

        #[test]
        fn flags_iter_is_fused_and_bounded() {
            let mut it = InitFlags::from_bits_retain(u32::MAX).iter();
            assert_eq!(it.size_hint(), (0, Some(6)));
            assert_eq!(it.by_ref().count(), 6);
            assert_eq!(it.size_hint(), (0, Some(0)));
            assert_eq!(it.next(), None);
        }

        #[test]
        fn unnamed_flag_reports_its_value() {
            let e = InitFlag::try_from(4).unwrap_err();
            assert_eq!(e.raw(), 4);
            assert_eq!(
                e.to_string(),
                "libgit2 repository init flag 0x4 is not named by git2/repository.h"
            );
            assert_eq!(InitFlag::try_from(0x20), Ok(InitFlag::ExternalTemplate));
        }

        #[test]
        fn modes_name_the_constants_and_keep_custom_bits() {
            assert_eq!(InitMode::from_raw(0), InitMode::SharedUmask);
            assert_eq!(InitMode::from_raw(0o2775), InitMode::SharedGroup);
            assert_eq!(InitMode::from_raw(0o2777), InitMode::SharedAll);
            assert_eq!(InitMode::default(), InitMode::SharedUmask);
            // The group mode without its setgid bit is a custom mode, not the group's.
            let custom = InitMode::from(0o775);
            let InitMode::Custom(mode) = custom else {
                panic!("{custom:?}")
            };
            assert_eq!(mode.bits(), 0o775);
            assert_eq!(u32::from(custom), 0o775);
            for raw in [1, 0o700, 0o2776, 0o7777, u32::MAX] {
                assert_eq!(InitMode::from_raw(raw).raw(), raw);
                assert!(matches!(InitMode::from_raw(raw), InitMode::Custom(_)));
            }
        }
    }

    mod items_and_open_flags {
        use super::super::*;

        #[test]
        fn items_are_indexed_by_value() {
            for (i, item) in Item::ALL.into_iter().enumerate() {
                assert_eq!(item.raw() as usize, i, "{item:?}");
                assert_eq!(Item::from_raw(item.raw()), Some(item));
                assert_eq!(Item::try_from(item.raw()), Ok(item));
                assert_eq!(ffi::git_repository_item_t::from(item), item.raw());
            }
            // `Ord` is the C value order.
            assert!(Item::ALL.windows(2).all(|w| w[0] < w[1]));
        }

        #[test]
        fn item_rejects_the_sentinel_and_beyond() {
            for raw in [
                ffi::GIT_REPOSITORY_ITEM__LAST,
                ffi::GIT_REPOSITORY_ITEM__LAST + 1,
                u32::MAX,
            ] {
                assert_eq!(Item::from_raw(raw), None);
                let err = Item::try_from(raw).unwrap_err();
                assert_eq!(err.raw(), raw);
                assert_eq!(
                    err.to_string(),
                    format!("libgit2 repository item {raw} is not named by git2/repository.h")
                );
            }
        }

        #[test]
        fn open_flag_bits_are_distinct_single_bits() {
            let mut seen = 0;
            for (i, flag) in OpenFlag::ALL.into_iter().enumerate() {
                assert_eq!(flag.raw(), 1 << i, "{flag:?}");
                assert_eq!(flag.bit() & seen, 0);
                seen |= flag.bit();
            }
            assert_eq!(OpenFlags::NAMED, seen);
            assert_eq!(OpenFlags::NAMED, 0x1f);
        }

        #[test]
        fn open_flag_conversions_round_trip() {
            for flag in OpenFlag::ALL {
                assert_eq!(OpenFlag::from_raw(flag.raw()), Some(flag));
                assert_eq!(OpenFlag::try_from(flag.raw()), Ok(flag));
                assert_eq!(ffi::git_repository_open_flag_t::from(flag), flag.raw());
            }
            for raw in [0, 3, 1 << 5, u32::MAX] {
                assert_eq!(OpenFlag::from_raw(raw), None);
                let err = OpenFlag::try_from(raw).unwrap_err();
                assert_eq!(err.raw(), raw);
                assert_eq!(
                    err.to_string(),
                    format!("libgit2 repository open flag {raw} is not named by git2/repository.h")
                );
            }
        }

        #[test]
        fn open_flags_build_and_take_apart() {
            let mut set = OpenFlags::default();
            assert!(set.is_empty());
            assert_eq!(set, OpenFlags::empty());
            set |= OpenFlag::Bare;
            set.insert(OpenFlag::NoSearch);
            assert_eq!(set.bits(), 0b101);
            assert_eq!(set, OpenFlag::NoSearch | OpenFlag::Bare);
            assert_eq!(set, OpenFlags::from(OpenFlag::Bare) | OpenFlag::NoSearch);
            assert_eq!(
                set.iter().collect::<Vec<_>>(),
                [OpenFlag::NoSearch, OpenFlag::Bare]
            );
            set.remove(OpenFlag::NoSearch);
            set.remove(OpenFlag::NoSearch);
            assert_eq!(set.into_iter().collect::<Vec<_>>(), [OpenFlag::Bare]);
            let mut more = OpenFlags::from_bits_retain(1 << 7);
            more |= set;
            assert_eq!(more.bits(), 1 << 7 | 0b100);
            assert_eq!(more.unnamed_bits(), 1 << 7);
            assert_eq!(
                OpenFlag::ALL.into_iter().collect::<OpenFlags>().bits(),
                0x1f
            );
        }

        #[test]
        fn open_flags_iter_skips_unnamed_bits_and_fuses() {
            let mut iter = OpenFlags::from_bits_retain(u32::MAX).iter();
            assert_eq!(iter.size_hint(), (0, Some(5)));
            assert_eq!(iter.by_ref().count(), 5);
            assert_eq!(iter.size_hint(), (0, Some(0)));
            assert_eq!(iter.next(), None);
            assert_eq!(OpenFlags::from_bits_retain(!0x1f).iter().next(), None);
        }

        /// A formatter target refusing every write after the first `ok`.
        struct FailAfter {
            ok: usize,
        }

        impl fmt::Write for FailAfter {
            fn write_str(&mut self, _: &str) -> fmt::Result {
                self.ok = self.ok.checked_sub(1).ok_or(fmt::Error)?;
                Ok(())
            }
        }

        #[test]
        fn open_flags_debug_passes_on_a_refused_write() {
            use core::fmt::Write;
            let set = OpenFlags::from_bits_retain(1 << 5 | 0b11);
            let writes = {
                let mut all = FailAfter { ok: usize::MAX };
                write!(all, "{set:?}").unwrap();
                usize::MAX - all.ok
            };
            for ok in 0..writes {
                assert_eq!(write!(FailAfter { ok }, "{set:?}"), Err(fmt::Error), "{ok}");
            }
        }

        #[test]
        fn open_flags_debug_names_flags_then_unnamed_bits() {
            assert_eq!(format!("{:?}", OpenFlags::empty()), "OpenFlags {}");
            assert_eq!(
                format!("{:?}", OpenFlag::NoSearch | OpenFlag::Bare),
                "OpenFlags {NoSearch | Bare}"
            );
            assert_eq!(
                format!("{:?}", OpenFlags::from_bits_retain(1 << 5 | 1)),
                "OpenFlags {NoSearch, 0x20}"
            );
            assert_eq!(
                format!("{:?}", OpenFlags::from_bits_retain(1 << 5)),
                "OpenFlags {0x20}"
            );
        }
    }

    mod state {
        use super::super::*;
        use std::collections::HashSet;
        use std::error::Error;

        /// Every `git_repository_state_t` constant, beside the variant that must carry it.
        const STATES: [(ffi::git_repository_state_t, RepositoryState); 12] = [
            (ffi::GIT_REPOSITORY_STATE_NONE, RepositoryState::Clean),
            (ffi::GIT_REPOSITORY_STATE_MERGE, RepositoryState::Merge),
            (ffi::GIT_REPOSITORY_STATE_REVERT, RepositoryState::Revert),
            (
                ffi::GIT_REPOSITORY_STATE_REVERT_SEQUENCE,
                RepositoryState::RevertSequence,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_CHERRYPICK,
                RepositoryState::CherryPick,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_CHERRYPICK_SEQUENCE,
                RepositoryState::CherryPickSequence,
            ),
            (ffi::GIT_REPOSITORY_STATE_BISECT, RepositoryState::Bisect),
            (ffi::GIT_REPOSITORY_STATE_REBASE, RepositoryState::Rebase),
            (
                ffi::GIT_REPOSITORY_STATE_REBASE_INTERACTIVE,
                RepositoryState::RebaseInteractive,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_REBASE_MERGE,
                RepositoryState::RebaseMerge,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX,
                RepositoryState::ApplyMailbox,
            ),
            (
                ffi::GIT_REPOSITORY_STATE_APPLY_MAILBOX_OR_REBASE,
                RepositoryState::ApplyMailboxOrRebase,
            ),
        ];

        /// Every value around the declared ones, and the extremes; `u32::MAX` is the
        /// `-1` `git_repository_state` returns on failure, read as the enum's type.
        fn probes() -> impl Iterator<Item = ffi::git_repository_state_t> {
            (0..=300).chain([u32::MAX - 1, u32::MAX])
        }

        #[test]
        fn variants_carry_their_c_values() {
            for (raw, state) in STATES {
                assert_eq!(state.raw(), raw);
                assert_eq!(ffi::git_repository_state_t::from(state), raw);
                assert_eq!(RepositoryState::from_raw(raw), Some(state));
                assert_eq!(RepositoryState::try_from(raw), Ok(state));
            }
            let distinct: HashSet<_> = STATES.iter().map(|(_, s)| *s).collect();
            assert_eq!(distinct.len(), STATES.len());
        }

        #[test]
        fn unnamed_values_are_refused_with_their_value() {
            for raw in probes() {
                match STATES.iter().find(|(r, _)| *r == raw) {
                    Some((_, state)) => assert_eq!(RepositoryState::from_raw(raw), Some(*state)),
                    None => {
                        assert_eq!(RepositoryState::from_raw(raw), None);
                        let err = RepositoryState::try_from(raw).unwrap_err();
                        assert_eq!(err, UnnamedRepositoryState(raw));
                        assert_eq!(err.raw(), raw);
                        assert_eq!(
                            err.to_string(),
                            format!(
                                "libgit2 repository state {raw} is not named by git2/repository.h"
                            )
                        );
                        assert!(err.source().is_none());
                    }
                }
            }
        }

        #[test]
        fn only_clean_is_clean() {
            for (raw, state) in STATES {
                assert_eq!(state.is_clean(), raw == ffi::GIT_REPOSITORY_STATE_NONE);
            }
        }

        #[test]
        fn option_niche_never_aliases_a_variant() {
            // `Option<RepositoryState>` stores `None` in a value no variant uses.
            assert_eq!(
                size_of::<Option<RepositoryState>>(),
                size_of::<ffi::git_repository_state_t>()
            );
            for raw in probes() {
                let state = core::hint::black_box(RepositoryState::from_raw(raw));
                assert_eq!(state.map(RepositoryState::raw).unwrap_or(raw), raw);
            }
        }

        #[test]
        fn debug_names_the_variant() {
            assert_eq!(format!("{:?}", RepositoryState::Clean), "Clean");
            assert_eq!(
                format!("{:?}", RepositoryState::CherryPickSequence),
                "CherryPickSequence"
            );
            assert_eq!(
                format!("{:?}", UnnamedRepositoryState(12)),
                "UnnamedRepositoryState(12)"
            );
        }
    }

    mod repository_owner {
        use super::super::*;

        use std::process::Command;
        use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

        use crate::libgit2::util::sys::alloc::{Alloc, Allocator};

        use std::ffi::CString;
        use std::fs;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        /// A scratch directory of its own under the temp dir, removed when dropped.
        struct Scratch {
            path: PathBuf,
            c_path: CString,
        }

        impl Scratch {
            fn new(tag: &str) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "crustify-repository-unit-{}-{tag}",
                    std::process::id()
                ));
                let _ = fs::remove_dir_all(&path);
                let c_path = CString::new(path.as_os_str().as_bytes()).unwrap();
                Self { path, c_path }
            }

            /// A bare repository whose references live in a reftable, with a file where
            /// its `reftable` directory goes: `git_refdb_init` cannot make the directory,
            /// after `git_repository_init` opened the repository.
            fn unmakeable_reftable(tag: &str) -> Self {
                let this = Self::new(tag);
                fs::create_dir_all(this.path.join("objects")).unwrap();
                fs::create_dir_all(this.path.join("refs")).unwrap();
                fs::write(this.path.join("HEAD"), "ref: refs/heads/main\n").unwrap();
                fs::write(
                    this.path.join("config"),
                    "[core]\n\trepositoryformatversion = 1\n\tbare = true\n\
                     [extensions]\n\trefstorage = reftable\n",
                )
                .unwrap();
                fs::write(this.path.join("reftable"), "not a directory\n").unwrap();
                this
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.path);
            }
        }

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// Runs `body` alone in a child process, where nothing has initialised libgit2.
        fn isolated(test: &str, body: fn()) {
            let name = test.split_once("::").map_or(test, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                body();
                return;
            }
            let out = Command::new(std::env::current_exe().unwrap())
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

        static REFUSING: AtomicBool = AtomicBool::new(false);

        unsafe extern "C" {
            fn malloc(n: usize) -> *mut core::ffi::c_void;
            fn realloc(p: *mut core::ffi::c_void, n: usize) -> *mut core::ffi::c_void;
            fn free(p: *mut core::ffi::c_void);
        }

        /// libc's allocator, refusing everything while `REFUSING` is set.
        struct Refusing;

        // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
        unsafe impl Alloc for Refusing {
            fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
                if REFUSING.load(SeqCst) {
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
                if REFUSING.load(SeqCst) {
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

        /// Installs [`Refusing`] as libgit2's allocator, before anything is allocated.
        fn install_refusing() {
            let allocator = Allocator::new::<Refusing>();
            // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with
            // every member set; libgit2 copies it before anything is allocated.
            let rc = unsafe {
                ffi::git_libgit2_opts(
                    ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                    allocator.as_ref().as_ptr(),
                )
            };
            assert_eq!(rc, 0);
        }

        /// When libgit2 cannot be initialised, every constructor fails without calling
        /// it, with the error `last` synthesises, and the policy builds nothing.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn constructors_refuse_without_a_runtime() {
            isolated(
                concat!(module_path!(), "::constructors_refuse_without_a_runtime"),
                || {
                    install_refusing();
                    REFUSING.store(true, SeqCst);
                    let new = Repository::new().unwrap_err();
                    REFUSING.store(false, SeqCst);
                    assert!(!runtime_pinned(), "libgit2 could not initialise");
                    assert!(RepositoryBox::new().is_none());
                    let results = [
                        new,
                        Repository::with_oid_type(OidType::Sha256).unwrap_err(),
                        Repository::open(c".").unwrap_err(),
                        Repository::open_bare(c".").unwrap_err(),
                        Repository::init(c".", true).unwrap_err(),
                    ];
                    for error in results {
                        assert_eq!(error.code(), Some(ErrorCode::Generic));
                        assert_eq!(error.class(), Some(ErrorClass::Invalid));
                        assert_eq!(error.message(), c"library has not been initialized");
                    }
                },
            );
        }

        /// An allocation failure fails `git_repository_new` and `_new_ext` with the
        /// out-of-memory error, and the policy's `CNew` with `None`.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn constructors_report_running_out_of_memory() {
            isolated(
                concat!(
                    module_path!(),
                    "::constructors_report_running_out_of_memory"
                ),
                || {
                    install_refusing();
                    // A failure first, so the thread's error state exists before
                    // allocation stops (else the error read is the thread-state one).
                    assert!(Repository::open(c"").is_err());
                    REFUSING.store(true, SeqCst);
                    let none = RepositoryBox::new().is_none();
                    let new = Repository::new().unwrap_err();
                    let with = Repository::with_oid_type(OidType::Sha1).unwrap_err();
                    REFUSING.store(false, SeqCst);
                    assert!(none);
                    for error in [new, with] {
                        assert_eq!(error.code(), Some(ErrorCode::Generic));
                        assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                    }
                    // Allocation works again.
                    drop(Repository::new().unwrap());
                },
            );
        }

        /// `git_repository_is_bare` and `git_repository_oid_type` of a live repository.
        fn bare_and_type(repo: &RepositoryBox) -> (c_int, ffi::git_oid_t) {
            let raw = repo.as_ref().as_ptr();
            // SAFETY: `raw` addresses the box's live repository, only read by both calls.
            unsafe {
                (
                    ffi::git_repository_is_bare(raw),
                    ffi::git_repository_oid_type(raw.cast_mut()),
                )
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn new_builds_a_bare_sha1_repository_without_paths() {
            let repo = Repository::new().unwrap();
            assert_eq!(bare_and_type(&repo), (1, ffi::GIT_OID_SHA1));
            // SAFETY: the box's live repository, only read.
            let path = unsafe { ffi::git_repository_path(repo.as_ref().as_ptr()) };
            assert!(path.is_null());
        }

        /// The policy's `CNew` builds the repository `Repository::new` builds.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn the_policy_constructs_what_new_does() {
            let repo = RepositoryBox::new().unwrap();
            assert_eq!(bare_and_type(&repo), (1, ffi::GIT_OID_SHA1));
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn with_oid_type_records_the_type() {
            for kind in [OidType::Sha1, OidType::Sha256] {
                let repo = Repository::with_oid_type(kind).unwrap();
                assert_eq!(bare_and_type(&repo), (1, kind.raw()));
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn an_error_keeps_negative_codes_and_reports_positive_ones_as_generic() {
            assert!(runtime_pinned());
            let not_found = RepositoryError::from_libgit2(ffi::GIT_ENOTFOUND);
            assert_eq!(not_found.code(), Some(ErrorCode::NotFound));
            assert_eq!(not_found.raw_code(), ffi::GIT_ENOTFOUND);
            let unnamed = RepositoryError::from_libgit2(-1000);
            assert_eq!(unnamed.code(), None);
            assert_eq!(unnamed.raw_code(), -1000);
            let positive = RepositoryError::from_libgit2(1);
            assert_eq!(positive.code(), Some(ErrorCode::Generic));
            assert_eq!(positive.raw_code(), -1);
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn an_error_formats_its_code_and_message() {
            let error = Repository::open(c"").unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::NotFound));
            assert_eq!(error.class(), Some(ErrorClass::Os));
            assert_eq!(error.raw_class(), ErrorClass::Os.raw());
            let message = "failed to resolve path '': No such file or directory";
            assert_eq!(error.message().to_str(), Ok(message));
            assert_eq!(error.to_string(), message);
            let debug = format!("{error:?}");
            assert!(
                debug.starts_with("RepositoryError { code: NotFound, error: "),
                "{debug}"
            );
            let unnamed = RepositoryError {
                code: -1000,
                error: error.error().clone(),
            };
            assert!(format!("{unnamed:?}").starts_with("RepositoryError { code: -1000,"));
            let buf: ErrorBuf = error.clone().into();
            assert_eq!(&buf, error.error());
        }

        /// The adoption plumbing releases a repository C left in the slot of a failed
        /// call, after the error is copied (LSan flags a leak, ASan a double free).
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn constructed_releases_a_repository_left_by_a_failure() {
            assert!(runtime_pinned());
            let mut out = ptr::null_mut();
            // SAFETY: `out` is a writable slot; the runtime is pinned.
            assert_eq!(unsafe { ffi::git_repository_new(&mut out) }, 0);
            assert!(!out.is_null());
            // SAFETY: `out` is the fresh repository just made, owned here alone.
            let repo = unsafe { RepositoryBox::from_c(out) };
            let error = constructed(ffi::GIT_EEXISTS, repo).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Exists));
            let mut out = ptr::null_mut();
            // SAFETY: `out` is a writable slot; the runtime is pinned.
            assert_eq!(unsafe { ffi::git_repository_new(&mut out) }, 0);
            // SAFETY: as above.
            let repo = unsafe { RepositoryBox::from_c(out) };
            let error = constructed(1, repo).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Generic));
        }

        /// An empty slot is a failure even where the code says success.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn constructed_refuses_a_null_slot() {
            assert!(runtime_pinned());
            let error = constructed(0, None).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Generic));
        }

        /// Corrected C behaviour: `git_repository_init` returns 1 and the repository it
        /// opened when the reftable backend cannot make its directory
        /// (`refdb_reftable_init` assigns `mkdir(..) < 0` to its error); the wrapper reports
        /// the failure, with libgit2's error, and releases that repository.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn init_reports_the_reftable_directory_failure() {
            let expected = |scratch: &Scratch| {
                format!(
                    "failed to make directory '{}': directory exists",
                    scratch.path.join("reftable").display()
                )
            };

            let c_side = Scratch::unmakeable_reftable("reftable-c");
            assert!(runtime_pinned());
            let mut out = ptr::null_mut();
            // SAFETY: `out` is a writable slot and the path NUL-terminated; the runtime is
            // pinned.
            let rc = unsafe { ffi::git_repository_init(&mut out, c_side.c_path.as_ptr(), 1) };
            let c_error = last();
            // SAFETY: C leaves the repository it opened in `out`, owned here; freed once.
            unsafe { ffi::git_repository_free(out) };
            assert_eq!(rc, 1);
            assert!(!out.is_null());
            assert_eq!(c_error.message().to_str(), Ok(&*expected(&c_side)));

            let rust_side = Scratch::unmakeable_reftable("reftable-rust");
            let error = Repository::init(&rust_side.c_path, true).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Generic));
            assert_eq!(error.class(), Some(ErrorClass::Filesystem));
            assert_eq!(error.message().to_str(), Ok(&*expected(&rust_side)));
        }
    }

    mod init_options {
        use super::super::*;
        use std::ffi::CString;

        /// `GIT_REPOSITORY_INIT_OPTIONS_INIT`, built in Rust so Miri can run the
        /// accessors; `new_matches_the_template` ties it to
        /// `git_repository_init_options_init`.
        fn template<'o>() -> InitOptions<'o> {
            let raw = ffi::git_repository_init_options {
                version: ffi::GIT_REPOSITORY_INIT_OPTIONS_VERSION,
                flags: 0,
                mode: 0,
                workdir_path: ptr::null(),
                description: ptr::null(),
                template_path: ptr::null(),
                initial_head: ptr::null(),
                origin_url: ptr::null(),
                oid_type: 0,
                refdb_type: 0,
            };
            InitOptions(raw, PhantomData)
        }

        /// The raw struct's fields, strings as their addresses.
        fn raw_fields(o: &InitOptions<'_>) -> [usize; 10] {
            let r = &o.0;
            [
                r.version as usize,
                r.flags as usize,
                r.mode as usize,
                r.workdir_path as usize,
                r.description as usize,
                r.template_path as usize,
                r.initial_head as usize,
                r.origin_url as usize,
                r.oid_type as usize,
                r.refdb_type as usize,
            ]
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn new_matches_the_template() {
            let options = InitOptions::new();
            assert_eq!(raw_fields(&options), raw_fields(&template()));
            assert_eq!(raw_fields(&InitOptions::default()), raw_fields(&options));
            assert_eq!(options.as_ref().version(), InitOptions::VERSION);
        }

        #[test]
        fn defaults_read_as_documented() {
            let options = template();
            let o = options.as_ref();
            assert_eq!(o.version(), 1);
            assert_eq!(o.flags(), InitFlags::empty());
            assert_eq!(o.mode(), InitMode::SharedUmask);
            assert_eq!(o.workdir_path(), None);
            assert_eq!(o.description(), None);
            assert_eq!(o.template_path(), None);
            assert_eq!(o.initial_head(), None);
            assert_eq!(o.origin_url(), None);
            assert_eq!((o.oid_type(), o.refdb_type()), (None, None));
            assert_eq!(
                format!("{options:?}"),
                "InitOptions { version: 1, flags: InitFlags {}, mode: SharedUmask, \
                 workdir_path: None, description: None, template_path: None, \
                 initial_head: None, origin_url: None, oid_type: None, refdb_type: None }"
            );
            assert_eq!(
                o.as_ptr() as usize % align_of::<ffi::git_repository_init_options>(),
                0
            );
        }

        #[test]
        fn each_setter_writes_its_field_only() {
            let s: Vec<CString> = ["work", "desc", "tpl", "trunk", "url"]
                .iter()
                .map(|s| CString::new(*s).unwrap())
                .collect();
            let mut options = template();
            let mut before = raw_fields(&options);
            // Index of the field each step must change, and the step.
            type Step<'s> = (usize, Box<dyn Fn(&mut InitOptionsMut<'_, 's>) + 's>);
            let steps: Vec<Step<'_>> = vec![
                (0, Box::new(|m| m.set_version(7))),
                (
                    1,
                    Box::new(|m| m.set_flags(InitFlag::Bare | InitFlag::Mkpath)),
                ),
                (2, Box::new(|m| m.set_mode(InitMode::SharedAll))),
                (3, Box::new(|m| m.set_workdir_path(Some(&s[0])))),
                (4, Box::new(|m| m.set_description(Some(&s[1])))),
                (5, Box::new(|m| m.set_template_path(Some(&s[2])))),
                (6, Box::new(|m| m.set_initial_head(Some(&s[3])))),
                (7, Box::new(|m| m.set_origin_url(Some(&s[4])))),
                (8, Box::new(|m| m.set_oid_type(Some(OidType::Sha256)))),
                (9, Box::new(|m| m.set_refdb_type(Some(RefdbType::Reftable)))),
            ];
            for (field, step) in &steps {
                step(&mut options.as_mut());
                let after = raw_fields(&options);
                for i in 0..10 {
                    assert_eq!(
                        after[i] != before[i],
                        i == *field,
                        "field {i}, step {field}"
                    );
                }
                before = after;
            }
            let raw = options.0;
            assert_eq!((raw.version, raw.flags, raw.mode), (7, 0x11, 0o2777));
            assert_eq!(raw.workdir_path, s[0].as_ptr());
            assert_eq!(raw.origin_url, s[4].as_ptr());
            assert_eq!(
                (raw.oid_type, raw.refdb_type),
                (ffi::GIT_OID_SHA256, ffi::GIT_REFDB_REFTABLE)
            );
            let o = options.as_ref();
            assert_eq!(
                o.to_options().0.initial_head,
                s[3].as_ptr(),
                "a copy keeps the borrowed pointers"
            );
            // Clearing stores NULL and 0 again.
            let mut m = options.as_mut();
            m.set_workdir_path(None);
            m.set_description(None);
            m.set_template_path(None);
            m.set_initial_head(None);
            m.set_origin_url(None);
            m.set_oid_type(None);
            m.set_refdb_type(None);
            let raw = options.0;
            assert!(raw.workdir_path.is_null() && raw.description.is_null());
            assert!(raw.template_path.is_null() && raw.initial_head.is_null());
            assert!(raw.origin_url.is_null());
            assert_eq!((raw.oid_type, raw.refdb_type), (0, 0));
        }

        #[test]
        fn getters_read_what_c_may_store() {
            // Values a C caller may leave in the struct: unnamed flag bits and modes
            // round-trip; unnamed types read as `None`, the default's reading.
            let mut options = template();
            options.0.flags = u32::MAX;
            options.0.mode = 0o755;
            options.0.oid_type = 3;
            options.0.refdb_type = 3;
            let o = options.as_ref();
            assert_eq!(o.flags().bits(), u32::MAX);
            assert_eq!(o.mode(), InitMode::from_raw(0o755));
            assert_eq!(o.mode().raw(), 0o755);
            assert_eq!((o.oid_type(), o.refdb_type()), (None, None));
            options.0.oid_type = ffi::GIT_OID_SHA1;
            options.0.refdb_type = ffi::GIT_REFDB_FILES;
            let o = options.as_ref();
            assert_eq!(
                (o.oid_type(), o.refdb_type()),
                (Some(OidType::Sha1), Some(RefdbType::Files))
            );
        }

        #[test]
        fn reborrows_and_debug_share_the_options() {
            let mut options = template();
            let mut m = options.as_mut();
            m.as_mut().set_initial_head(Some(c"main"));
            assert_eq!(format!("{m:?}"), format!("{:?}", m.as_ref()));
            let r = m.as_ref();
            assert_eq!(r.initial_head(), Some(c"main"));
            assert!(format!("{options:?}").contains(r#"initial_head: Some("main")"#));
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn with_oid_type_goes_through_new_options() {
            for kind in [OidType::Sha1, OidType::Sha256] {
                let repo = Repository::with_oid_type(kind).unwrap();
                // SAFETY: a live repository, only read.
                let raw = unsafe { ffi::git_repository_oid_type(repo.as_c_ptr()) };
                assert_eq!(raw, kind.raw());
            }
        }
    }
}
