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
//! `git_repository_init_ext` ([`Repository::init_ext`]) writes to its options, ORing
//! internal bits into their flags, so it takes them exclusively ([`InitOptionsMut`]);
//! the wrapper clears those bits before each call, where libgit2 would read the ones a
//! previous call left as describing this one.
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
//!
//! The routines over a repository go on its handles: [`RepositoryRef`] for those that
//! read it, or only fill its caches and write through the counted objects it shares
//! (its paths and object id type, the odb and refdb it lends, its state, its message,
//! its `HEAD`); [`RepositoryMut`] for those that replace a string it lends or a part it
//! owns (its identity, namespace and working directory, and the setters of
//! `git2/sys/repository.h`). A repository without a git directory ([`Repository::new`])
//! is refused by the routines that would build a path inside it, where libgit2 resolves
//! the path against the current directory or dereferences it. A
//! [`MergeheadCallback`] is the closure `git_repository_mergehead_foreach` calls with
//! each id of `MERGE_HEAD`.
//!
//! A string a getter lends ([`commondir`](RepositoryRef::commondir),
//! [`namespace`](RepositoryRef::namespace), [`ident`](RepositoryRef::ident)) lives as
//! long as the shared borrow. The counted index ([`IndexArc`]), a config snapshot and
//! the commits of [`commit_parents`](RepositoryRef::commit_parents) outlive the
//! repository; the counted config ([`ConfigArc`], whose file backends keep the
//! repository for their `includeIf` conditions) and references ([`ReferenceBox`])
//! borrow it. [`discover`] finds a repository without
//! opening it, and a [`FetchheadForeachCallback`] is the closure
//! [`for_each_fetchhead`](RepositoryRef::for_each_fetchhead) calls with each
//! [`FetchheadEntry`].
//!
//! A repository built without paths ([`Repository::new`], [`Repository::wrap_odb`]) has
//! no git directory, common directory or working directory. libgit2 resolves some
//! paths against those regardless, so against the process's working directory; the
//! wrappers whose routines would then write or remove files there refuse such a
//! repository instead ([`RepositoryMut::cleanup_state`], and
//! [`Worktree::lookup`](crate::libgit2::repository::worktree::Worktree::lookup), whose
//! routine would read a NULL path).

use core::any::Any;
use core::ffi::{CStr, c_char, c_int, c_uint, c_void};
use core::fmt;
use core::marker::PhantomData;
use core::mem::MaybeUninit;
use core::num::NonZero;
use core::ops::{BitOr, BitOrAssign, ControlFlow};
use core::ptr::{self, NonNull, addr_of, addr_of_mut};
use std::panic::{self, AssertUnwindSafe};

use ffibox::{CBorrowedPtr, CBox, CCell, CNew, CZeroable, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::config::config::{ConfigArc, ConfigBox};
use crate::libgit2::core::buffer::{Buf, BufVal};
use crate::libgit2::core::types::ObjectType;
use crate::libgit2::index::index::IndexArc;
use crate::libgit2::object::commit::{CommitArray, CommitArrayVal};
use crate::libgit2::odb::odb::OdbArc;
use crate::libgit2::odb::sys::odb_backend::OdbDataVec;
use crate::libgit2::oid::oid::{Oid, OidRef, OidType};
use crate::libgit2::refs::annotated_commit::AnnotatedCommitRef;
use crate::libgit2::refs::refdb::{RefdbArc, RefdbType};
use crate::libgit2::refs::refs::{ReferenceBox, UnattachedReference, UnattachedReferenceBox};
use crate::libgit2::repository::sys::repository::NewOptions;
use crate::libgit2::repository::worktree::WorktreeRef;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, ErrorCode, last};
use crate::libgit2::util::sys::errors::{clear, runtime_pinned, set, set_oom};
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
/// it is given, and reads them back on the next call with those options, which
/// [`Repository::init_ext`] prevents by clearing them before each call.
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
    /// the rest; [`Repository::init_ext`] clears the internal ones before its call.
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

    /// Sets the working directory. `None` means the repository directory's parent,
    /// for a repository path ending in `.git`; to any other path libgit2 then appends
    /// `.git`, so that the path given is the working directory. With one, the
    /// repository path is used as given; libgit2 resolves a relative working directory
    /// against it and, unless it is the repository directory's parent, writes a `.git`
    /// file there linking to the repository and records it as `core.worktree`. Ignored
    /// for a bare repository. Read during each call, never kept.
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
    /// whatever the flags; an empty one copies nothing, a missing directory (or a
    /// file) falls back to the built-in template, and a path the file system refuses
    /// as invalid (an over-long component) fails the call with `GIT_EINVALIDSPEC`
    /// once the repository directory exists. Read during each call, never kept.
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
    /// libgit2 writes the name to `HEAD` without checking that it is a valid reference
    /// name. Read during each call, never kept.
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

/// The failure of a repository constructor or routine: the code it returned, and the
/// error it left as the thread's last libgit2 error, copied out.
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
    pub(crate) fn from_libgit2(rc: c_int) -> Self {
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

/// The bits `git_repository_init_ext` ORs into its options' flags for its own use
/// (`GIT_REPOSITORY_INIT__HAS_DOTGIT`, `__NATURAL_WD`, `__IS_REINIT`, private to
/// `repository.c`), and reads back from them.
const INIT_INTERNAL_BITS: u32 = 1 << 16 | 1 << 17 | 1 << 18;

/// The repository a constructor returned `rc` for, adopted from its out slot.
///
/// `git_repository_init` can fail after it opened the repository, leaving it in the
/// slot (and then may return a positive code, see [`Repository::init`]); it is
/// released here, after the error is copied out. A slot left NULL on success would be
/// a libgit2 bug, reported as the thread's error.
pub(crate) fn constructed(
    rc: c_int,
    repo: Option<RepositoryBox>,
) -> Result<RepositoryBox, RepositoryError> {
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
pub(crate) fn unpinned() -> RepositoryError {
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
        let mut opts = NewOptions::new();
        opts.as_mut().set_oid_type(Some(oid_type));
        Self::new_ext(opts.as_ref())
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

    /// Creates a repository at `path`, or reinitialises the one there, as `opts`
    /// directs, and opens it: [`init`](Self::init) with every option of
    /// [`InitOptions`] (`init(path, bare)` is this with [`InitFlag::Mkpath`], plus
    /// [`InitFlag::Bare`] when `bare`, and the other options at their defaults). `path`
    /// is the repository directory for a bare repository, or for one whose path ends in
    /// `.git` or that has a [working directory](InitOptionsMut::set_workdir_path) of its
    /// own; any other path is the working directory, the repository going in
    /// `path/.git`.
    ///
    /// Fails with libgit2's error: with [`ErrorCode::Exists`] when the path holds a
    /// repository and [`InitFlag::NoReinit`] is set, or when a directory on the way is
    /// a file; with `GIT_ERROR_INVALID` for options of a version libgit2 does not know.
    /// As for `init`, libgit2 can fail after opening the repository, which is then
    /// released here: when the `origin` remote of
    /// [`set_origin_url`](InitOptionsMut::set_origin_url) cannot be added (a
    /// reinitialised repository that has one fails with [`ErrorCode::Exists`] once it
    /// is otherwise reinitialised), or when its reference storage cannot be set up.
    ///
    /// libgit2 writes to the options: it ORs its own internal bits into their
    /// [`flags`](InitOptionsRef::flags) (a path ending in `.git`, a working directory
    /// that is its parent, a reinitialisation; see [`InitFlags`]), and reads them back
    /// on the next call, where they describe the previous path. This wrapper clears
    /// those three bits before each call, so the options can be reused: libgit2 itself
    /// would, for one, skip the `.git` link and `core.worktree` of a repository whose
    /// working directory is elsewhere after an init whose working directory was the
    /// parent. After the call the flags hold the bits this call set (none when the
    /// options were refused for their version), as they would after a first call in C.
    ///
    /// ```
    /// use libgit2::libgit2::repository::repository::{InitFlag, InitOptions, Repository};
    ///
    /// # if cfg!(miri) { return; }
    /// let dir = std::env::temp_dir().join(format!("init-ext-doc-{}", std::process::id()));
    /// let path = std::ffi::CString::new(dir.join("repo").to_str().unwrap()).unwrap();
    /// let mut opts = InitOptions::new();
    /// opts.as_mut().set_flags(InitFlag::Bare | InitFlag::Mkpath);
    /// opts.as_mut().set_initial_head(Some(c"trunk"));
    /// let repo = Repository::init_ext(&path, opts.as_mut()).unwrap();
    /// assert!(repo.as_ref().is_bare());
    /// assert!(repo.as_ref().is_head_unborn().unwrap());
    /// # drop(repo);
    /// # std::fs::remove_dir_all(&dir).unwrap();
    /// ```
    ///
    /// Wraps: git_repository_init_ext
    pub fn init_ext(
        path: &CStr,
        mut opts: InitOptionsMut<'_, '_>,
    ) -> Result<RepositoryBox, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        let flags = opts.as_ref().flags().bits() & !INIT_INTERNAL_BITS;
        opts.set_flags(InitFlags::from_bits_retain(flags));
        let opts = opts.as_mut_ptr();
        let mut out = ptr::null_mut();
        // The options' version is checked by libgit2; their object id type and reference
        // storage are 0 or named values (the typed setters).
        // SAFETY: `out` is a writable slot, NULL so a failure before the inner open
        // leaves it NULL; `path` NUL-terminated; `opts` the exclusive handle's live
        // options, whose flags libgit2 writes and whose strings (NULL or borrowed for
        // `'o`, which outlives the handle) it reads for the call only. Runtime pinned.
        let rc = unsafe { ffi::git_repository_init_ext(&mut out, path.as_ptr(), opts) };
        // SAFETY: `git_repository_init_ext` leaves `out` NULL, or writes the fresh
        // repository it opened, owned by the caller alone, whatever it returns.
        constructed(rc, unsafe { RepositoryBox::from_c(out) })
    }
}

impl Repository {
    /// A repository over the object database `odb`, with no paths, config, index or
    /// working directory: the repository API (object lookups, its object cache) over an
    /// odb built by hand, custom backends and all. Its objects are ids of the odb's
    /// type.
    ///
    /// The repository takes a reference to `odb` of its own and becomes its owner (see
    /// [`odb`](crate::libgit2::odb::odb#the-owning-repository)), so it accepts only an
    /// odb no repository owns yet, an `OdbArc<'static>`; the reference passed in is
    /// released once the repository holds its own.
    ///
    /// Fails only when libgit2 runs out of memory, or cannot be initialised.
    ///
    /// An odb a repository lent (`OdbArc<'r>`) is not accepted:
    ///
    /// ```compile_fail
    /// use libgit2::libgit2::odb::odb::OdbArc;
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// fn rewrap<'r>(odb: OdbArc<'r>) {
    ///     let _ = Repository::wrap_odb(odb);
    /// }
    /// ```
    ///
    /// Wraps: git_repository_wrap_odb
    pub fn wrap_odb(odb: OdbArc<'static>) -> Result<RepositoryBox, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot, NULL so that a failure (which leaves it
        // untouched) leaves it NULL; `odb` is a live odb no repository owns (`'static`),
        // of a type libgit2 names (every safe constructor gives one), which the new
        // repository takes a reference to and owns. The runtime is pinned.
        let rc = unsafe { ffi::git_repository_wrap_odb(&mut out, odb.as_c_ptr()) };
        // SAFETY: `git_repository_wrap_odb` writes a fresh repository the caller owns
        // alone only on success.
        let repo = constructed(rc, unsafe { RepositoryBox::from_c(out) });
        drop(odb);
        repo
    }
}

impl<'a> RepositoryRef<'a> {
    /// The repository's working directory, with a trailing slash; `None` for a bare
    /// repository, and for one without paths.
    ///
    /// The string is the repository's, borrowed for as long as the handle: libgit2
    /// replaces it only when the working directory is changed or the repository made
    /// bare, through a [`RepositoryMut`], which no shared borrow outlives. Nor does
    /// the string outlive the repository:
    ///
    /// ```compile_fail,E0505
    /// use core::ffi::CStr;
    /// use libgit2::libgit2::repository::repository::RepositoryBox;
    ///
    /// fn escape(repo: RepositoryBox) -> Option<&'static CStr> {
    ///     let workdir = repo.as_ref().workdir();
    ///     drop(repo);
    ///     workdir
    /// }
    /// ```
    ///
    /// Wraps: git_repository_workdir
    pub fn workdir(&self) -> Option<&'a CStr> {
        // SAFETY: a live repository on this thread; the routine only reads its
        // `is_bare` and `workdir` fields.
        let dir = unsafe { ffi::git_repository_workdir(self.as_ptr()) };
        // SAFETY: `workdir` is NULL or a NUL-terminated string the repository owns,
        // freed only with it, by `git_repository_set_workdir` or by
        // `git_repository_set_bare`, whose wrappers take a `RepositoryMut`, which the
        // `'a` borrow excludes; so it lives for `'a`.
        (!dir.is_null()).then(|| unsafe { CStr::from_ptr(dir) })
    }
}

impl RepositoryMut<'_> {
    /// Removes what a stopped merge, revert, cherry-pick, bisect, rebase or mailbox
    /// application left in the git directory: the `MERGE_HEAD`, `MERGE_MODE`,
    /// `MERGE_MSG` and `BISECT_LOG` files, the `rebase-merge/`, `rebase-apply/` and
    /// `sequencer/` directories, and the `REVERT_HEAD` and `CHERRY_PICK_HEAD`
    /// references, so that the state is [`RepositoryState::Clean`]. What is missing is
    /// skipped.
    ///
    /// Fails with libgit2's error when one of them cannot be removed (with
    /// `GIT_ERROR_OS` for a file, which libgit2 reports without an error of its own),
    /// and with [`ErrorCode::NotFound`] for a repository without paths, which has no
    /// git directory: libgit2 would remove those names from the process's working
    /// directory.
    ///
    /// It removes files through the exclusive handle alone:
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::repository::repository::RepositoryBox;
    ///
    /// fn cleanup(repo: &RepositoryBox) {
    ///     let _ = repo.as_ref().cleanup_state();
    /// }
    /// ```
    ///
    /// Wraps: git_repository_state_cleanup
    pub fn cleanup_state(&mut self) -> Result<(), RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        clear();
        // SAFETY: a live repository on this thread; the routine only reads its `gitdir`
        // field.
        if unsafe { ffi::git_repository_path(self.as_ref().as_ptr()) }.is_null() {
            set(ErrorClass::Invalid, c"path cannot exist in repository");
            return Err(RepositoryError::from_libgit2(ErrorCode::NotFound.raw()));
        }
        // SAFETY: a live repository on this thread, exclusively borrowed, with a git
        // directory the removals are relative to; its reference database may be loaded
        // and written. The runtime is pinned.
        let rc = unsafe { ffi::git_repository_state_cleanup(self.as_mut_ptr()) };
        if rc < 0 {
            if last().class() == Some(ErrorClass::None) {
                set(ErrorClass::Os, c"failed to remove a repository state file");
            }
            return Err(RepositoryError::from_libgit2(rc));
        }
        Ok(())
    }
}

// --- Repository routines -----------------------------------------------------------

/// The failure a repository without a git directory (one [`Repository::new`],
/// [`Repository::with_oid_type`] or `git_repository_wrap_odb` built) reports where a
/// routine would build a path inside it: `GIT_ENOTFOUND`, with the error libgit2's
/// `git_repository_item_path` sets for such a repository.
///
/// libgit2 itself joins the missing directory as an empty one, so those routines read,
/// write or delete files relative to the process's current directory instead.
pub(crate) fn pathless() -> RepositoryError {
    set(ErrorClass::Invalid, c"path cannot exist in repository");
    RepositoryError::from_libgit2(ErrorCode::NotFound.raw())
}

/// `Ok` for a routine's non-negative return, its failure otherwise.
pub(crate) fn status(rc: c_int) -> Result<(), RepositoryError> {
    if rc < 0 {
        Err(RepositoryError::from_libgit2(rc))
    } else {
        Ok(())
    }
}

/// The failure of a routine that returned `rc < 0`, with the thread's error, or with
/// `class` and `message` when the routine set none: the error state was cleared before
/// the call, so a `GIT_ERROR_NONE` error is the routine's silence, not an older error.
fn failed_quietly(rc: c_int, class: ErrorClass, message: &CStr) -> RepositoryError {
    if last().class() == Some(ErrorClass::None) {
        set(class, message);
    }
    RepositoryError::from_libgit2(rc)
}

/// `Ok` when `id` is of `repo`'s object id type, the refusal otherwise.
///
/// libgit2 formats an id into a loose object's path sized for the object database's
/// type, which is the repository's: a longer id overflows it, a shorter one leaves part
/// of it unwritten. The refusal is libgit2's own wording for the mismatch.
fn id_fits(repo: RepositoryRef<'_>, id: OidRef<'_>) -> Result<(), RepositoryError> {
    if u32::from(id.raw_kind()) == repo.oid_type().raw() {
        return Ok(());
    }
    set(
        ErrorClass::Invalid,
        c"specified object ID type does not match repository object ID type",
    );
    Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()))
}

impl Repository {
    /// Searches for a repository from `start` and opens it, as [`OpenFlags`] direct:
    /// by default `start/.git`, then `start` itself, then each parent directory in turn
    /// up to (excluding) a ceiling directory, staying on `start`'s file system.
    ///
    /// `ceiling_dirs` lists absolute directories, separated by `:`, that the search
    /// must not walk up into; `None` for none (or `$GIT_CEILING_DIRECTORIES`, under
    /// [`OpenFlag::FromEnv`]). `start` may be
    /// `None` only under [`OpenFlag::FromEnv`], which then opens `$GIT_DIR` without
    /// searching, or searches from the current directory when it is unset; any other
    /// `None` is refused before the call, with the error a release build of libgit2
    /// reports for it (`GIT_ERROR_INVALID`, "invalid argument: 'path'"), where a debug
    /// build aborts.
    ///
    /// Fails with [`ErrorCode::NotFound`] when the search finds no repository
    /// (GIT_ERROR_REPOSITORY, "could not find repository at '`start`'") or `start` does
    /// not resolve, with [`ErrorCode::Owner`] when the repository belongs to another
    /// user and is not a `safe.directory`, and otherwise with libgit2's error.
    ///
    /// Wraps: git_repository_open_ext
    pub fn open_ext(
        start: Option<&CStr>,
        flags: OpenFlags,
        ceiling_dirs: Option<&CStr>,
    ) -> Result<RepositoryBox, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        let start = start_path(start, flags)?.map_or(ptr::null(), CStr::as_ptr);
        let ceiling = ceiling_dirs.map_or(ptr::null(), CStr::as_ptr);
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `start` a NUL-terminated string, or NULL
        // only under `FROM_ENV`, which reads it from the environment instead; `ceiling`
        // NULL or a NUL-terminated string; both read for the call only. The runtime is
        // pinned.
        let rc = unsafe { ffi::git_repository_open_ext(&mut out, start, flags.bits(), ceiling) };
        // SAFETY: `git_repository_open_ext` NULLs `out` first and writes a fresh
        // repository the caller owns alone only on success, freeing its own on failure.
        constructed(rc, unsafe { RepositoryBox::from_c(out) })
    }

    /// Whether the search [`open_ext`](Self::open_ext) runs from `start` finds a
    /// repository, without opening it: libgit2's `git_repository_open_ext` with a NULL
    /// out slot, which stops after the search. The repository found is not checked
    /// further (its format, its owner), so `open_ext` can still fail on it.
    ///
    /// `false` when the search returns `GIT_ENOTFOUND` (no repository, or a `start`
    /// that does not resolve); other failures, an unparsable
    /// `$GIT_DISCOVERY_ACROSS_FILESYSTEM` under [`OpenFlag::FromEnv`] among them, are
    /// errors. `start` and `ceiling_dirs` are read as `open_ext` reads them.
    ///
    /// Wraps: git_repository_open_ext
    pub fn exists_ext(
        start: Option<&CStr>,
        flags: OpenFlags,
        ceiling_dirs: Option<&CStr>,
    ) -> Result<bool, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        let start = start_path(start, flags)?.map_or(ptr::null(), CStr::as_ptr);
        let ceiling = ceiling_dirs.map_or(ptr::null(), CStr::as_ptr);
        let (out, flags) = (ptr::null_mut(), flags.bits());
        // SAFETY: a NULL out slot, which libgit2 accepts as "only search"; `start` and
        // `ceiling` as in `open_ext`. The runtime is pinned.
        let rc = unsafe { ffi::git_repository_open_ext(out, start, flags, ceiling) };
        match rc {
            0.. => Ok(true),
            ffi::GIT_ENOTFOUND => Ok(false),
            rc => Err(RepositoryError::from_libgit2(rc)),
        }
    }

    /// Opens the repository of the working directory `worktree` checks out: the
    /// directory of its `.git` file, as [`open`](Self::open) opens it. The repository
    /// is independent of `worktree`, which may go first.
    ///
    /// Fails with libgit2's error when the directory holds no repository. A worktree
    /// whose recorded `gitdir` does not end in `.git` fails with `-1`, for which
    /// libgit2 sets no error: it is reported as `GIT_ERROR_WORKTREE`, "worktree gitdir
    /// does not end in '.git'".
    ///
    /// Wraps: git_repository_open_from_worktree
    pub fn open_from_worktree(worktree: WorktreeRef<'_>) -> Result<RepositoryBox, RepositoryError> {
        // A worktree is built by libgit2 only, so the runtime is pinned already.
        clear();
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `worktree` a live worktree whose
        // `gitlink_path` libgit2 only reads, keeping no pointer into it. The runtime was
        // pinned to build the worktree.
        let rc = unsafe {
            ffi::git_repository_open_from_worktree(&mut out, worktree.as_ptr().cast_mut())
        };
        // SAFETY: `git_repository_open_from_worktree` NULLs `out` first and writes a
        // fresh repository (from `git_repository_open`) the caller owns alone only on
        // success.
        let repo = unsafe { RepositoryBox::from_c(out) };
        if rc < 0 {
            drop(repo);
            return Err(failed_quietly(
                rc,
                ErrorClass::Worktree,
                c"worktree gitdir does not end in '.git'",
            ));
        }
        constructed(rc, repo)
    }
}

/// The start path `git_repository_open_ext` takes: `start`, or NULL under
/// [`OpenFlag::FromEnv`]; any other `None` is refused as a release build of libgit2
/// refuses it (`GIT_ASSERT_ARG(path)` in `git_fs_path_prettify`).
fn start_path(start: Option<&CStr>, flags: OpenFlags) -> Result<Option<&CStr>, RepositoryError> {
    match start {
        None if !flags.contains(OpenFlag::FromEnv) => {
            set(ErrorClass::Invalid, c"invalid argument: 'path'");
            Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()))
        }
        start => Ok(start),
    }
}

impl<'a> RepositoryRef<'a> {
    /// The repository's git directory, with a trailing `/`: the `.git` directory, or
    /// the repository itself when it is bare. A worktree's is its administrative
    /// directory under the main repository's `worktrees/`. `None` for a repository
    /// without one ([`Repository::new`], `git_repository_wrap_odb`).
    ///
    /// libgit2 sets it when it opens the repository and frees it with the repository,
    /// so it lives as long as the borrow of the repository:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::open(c".").unwrap();
    /// let path = repo.as_ref().path();
    /// drop(repo);
    /// let _ = path;
    /// ```
    ///
    /// Wraps: git_repository_path
    pub fn path(&self) -> Option<&'a CStr> {
        // SAFETY: a live repository, only read.
        let path = unsafe { ffi::git_repository_path(self.as_ptr()) };
        if path.is_null() {
            return None;
        }
        // SAFETY: `gitdir` is a NUL-terminated string the repository owns, assigned only
        // by the routines that open it and freed with it: no routine replaces it, so it
        // stays unchanged for `'a`, the borrow of the live repository.
        Some(unsafe { CStr::from_ptr(path) })
    }

    /// The type of the object ids the repository stores. Every repository has a named
    /// one: opening one whose `extensions.objectformat` libgit2 does not know fails.
    ///
    /// Wraps: git_repository_oid_type
    pub fn oid_type(&self) -> OidType {
        // SAFETY: a live repository, whose `oid_type` libgit2 only reads.
        let raw = unsafe { ffi::git_repository_oid_type(self.as_ptr().cast_mut()) };
        OidType::from_raw(raw).expect("a repository's object id type is always a named one")
    }

    /// One more reference to the repository's object database, loaded from its
    /// `objects` directory (and alternates) on first use.
    ///
    /// The repository owns the odb, which reads and writes through the repository's
    /// object cache: the reference borrows the repository (see
    /// [`Odb`](crate::libgit2::odb::odb::Odb)'s owning repository), and shares it
    /// with any other holder, on any thread, as long as the repository lives:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::open(c".").unwrap();
    /// let odb = repo.as_ref().odb().unwrap();
    /// drop(repo);
    /// drop(odb);
    /// ```
    ///
    /// Fails with `-1` and libgit2's error when the odb cannot be loaded: always for a
    /// repository without a git directory (`GIT_ERROR_INVALID`, "path cannot exist in
    /// repository").
    ///
    /// Wraps: git_repository_odb
    pub fn odb(&self) -> Result<OdbArc<'a>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `self` a live repository on this thread,
        // whose odb libgit2 loads and installs through it on first use, a cache fill
        // `RepositoryRef` allows.
        let rc = unsafe { ffi::git_repository_odb(&mut out, self.as_ptr().cast_mut()) };
        status(rc)?;
        // SAFETY: on success `out` is one new counted reference to the odb the
        // repository owns, released by `git_odb_free` as `OdbFree` does; the repository
        // outlives `'a`, which the arc carries.
        let odb = unsafe { OdbArc::from_c(out) };
        Ok(odb.expect("git_repository_odb succeeded without an odb"))
    }

    /// One more reference to the repository's reference database, opened with the
    /// backend its storage format names on first use.
    ///
    /// The refdb points back at the repository it was opened for, which the reference
    /// borrows (see [`Refdb`](crate::libgit2::refs::refdb::Refdb)):
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::open(c".").unwrap();
    /// let refdb = repo.as_ref().refdb().unwrap();
    /// drop(repo);
    /// drop(refdb);
    /// ```
    ///
    /// A repository
    /// without a git directory gets a files backend whose paths resolve against the
    /// current directory.
    ///
    /// Wraps: git_repository_refdb
    pub fn refdb(&self) -> Result<RefdbArc<'a>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `self` a live repository on this thread,
        // whose refdb libgit2 opens and installs through it on first use, a cache fill.
        let rc = unsafe { ffi::git_repository_refdb(&mut out, self.as_ptr().cast_mut()) };
        status(rc)?;
        // SAFETY: on success `out` is one new counted reference to the repository's
        // refdb, released by `git_refdb_free`; the repository the refdb points back at
        // is this one, live for `'a`, or one that a `set_refdb` caller vouched outlives
        // it.
        let db = unsafe { RefdbArc::from_c(out) };
        Ok(db.expect("git_repository_refdb succeeded without a refdb"))
    }

    /// The operation the repository is in the middle of, read from the marker files
    /// in its git directory (see [`RepositoryState`]).
    ///
    /// Fails with `-1` when libgit2 runs out of memory. A repository without a git
    /// directory is refused before the call, as a release build of libgit2 refuses it
    /// (`-1`, `GIT_ERROR_INVALID` "invalid argument: 'string'"), where a debug build
    /// aborts.
    ///
    /// Wraps: git_repository_state
    pub fn state(&self) -> Result<RepositoryState, RepositoryError> {
        if self.path().is_none() {
            set(ErrorClass::Invalid, c"invalid argument: 'string'");
            return Err(RepositoryError::from_libgit2(ErrorCode::Generic.raw()));
        }
        // SAFETY: a live repository with a git directory, on this thread; libgit2 may
        // open its refdb through it, a cache fill.
        let rc = unsafe { ffi::git_repository_state(self.as_ptr().cast_mut()) };
        status(rc)?;
        let state = RepositoryState::from_raw(rc as ffi::git_repository_state_t);
        Ok(state.expect("git_repository_state returned a state the header does not name"))
    }

    /// The prepared commit message of the operation in progress: the contents of the
    /// git directory's `MERGE_MSG`, as stored (no NUL is added or stripped).
    ///
    /// Fails with [`ErrorCode::NotFound`] when there is none (GIT_ERROR_OS, "could not
    /// access message file"), or with libgit2's error when it cannot be read. A
    /// repository without a git directory is refused with [`ErrorCode::NotFound`]
    /// (GIT_ERROR_INVALID, "path cannot exist in repository"): libgit2 would read
    /// `MERGE_MSG` in the current directory.
    ///
    /// Wraps: git_repository_message
    pub fn message(&self) -> Result<BufVal, RepositoryError> {
        if self.path().is_none() {
            return Err(pathless());
        }
        let mut buf = Buf::new();
        // SAFETY: `buf` is an empty buffer, which libgit2 fills; `self` a live
        // repository with a git directory, only read.
        let rc = unsafe {
            ffi::git_repository_message(buf.as_mut().as_mut_ptr(), self.as_ptr().cast_mut())
        };
        status(rc)?;
        Ok(buf)
    }

    /// Removes the git directory's `MERGE_MSG`.
    ///
    /// Fails with `-1` when it cannot be removed, its absence included. libgit2 sets
    /// no error for that failure: it is reported as `GIT_ERROR_OS`, "could not remove
    /// message file: `strerror`". A repository without a git directory is refused as
    /// [`message`](Self::message) refuses it: libgit2 would remove `MERGE_MSG` from the
    /// current directory.
    ///
    /// Wraps: git_repository_message_remove
    pub fn remove_message(&self) -> Result<(), RepositoryError> {
        if self.path().is_none() {
            return Err(pathless());
        }
        clear();
        // SAFETY: a live repository with a git directory, only read.
        let rc = unsafe { ffi::git_repository_message_remove(self.as_ptr().cast_mut()) };
        if rc < 0 {
            return Err(failed_quietly(
                rc,
                ErrorClass::Os,
                c"could not remove message file",
            ));
        }
        Ok(())
    }

    /// Points `HEAD` at `refname`, recording a reflog entry (`checkout: moving from
    /// <old> to <new>`).
    ///
    /// A branch (`refs/heads/...`), existing or not, makes `HEAD` a symbolic reference
    /// to it; any other existing reference detaches `HEAD` at the commit it peels to.
    /// Fails with [`ErrorCode::NotFound`] when `HEAD` itself is missing, or when
    /// `refname` names no branch and no existing reference; with `-1` when the branch
    /// is checked out in a linked worktree, or the reference does not peel to a commit.
    ///
    /// The write goes through the repository's reference database and object database,
    /// shared counted objects, so it takes the shared handle. A repository without a
    /// git directory reaches the files backend of [`refdb`](Self::refdb), which reads
    /// and writes `HEAD` relative to the current directory unless a reference database
    /// was installed.
    ///
    /// Wraps: git_repository_set_head
    pub fn set_head(&self, refname: &CStr) -> Result<(), RepositoryError> {
        let repo = self.as_ptr().cast_mut();
        // SAFETY: a live repository on this thread, whose refdb and odb libgit2 loads
        // through it as needed; `refname` a NUL-terminated string read for the call.
        let rc = unsafe { ffi::git_repository_set_head(repo, refname.as_ptr()) };
        status(rc)
    }

    /// Detaches `HEAD` at the commit `commit` names or peels to (a tag is peeled),
    /// recording a reflog entry named after the commit's id.
    ///
    /// Fails with [`ErrorCode::NotFound`] when `HEAD` is missing or the object is
    /// not in the repository, with libgit2's error when it does not peel to a commit.
    /// An id whose type is not the repository's is refused before the call, with
    /// GIT_ERROR_INVALID "specified object ID type does not match repository object ID
    /// type": libgit2 would format it into a loose object's path of the wrong length.
    ///
    /// Wraps: git_repository_set_head_detached
    pub fn set_head_detached(&self, commit: OidRef<'_>) -> Result<(), RepositoryError> {
        id_fits(*self, commit)?;
        // SAFETY: a live repository on this thread; `commit` a live id of the
        // repository's type, read for the call.
        let rc = unsafe {
            ffi::git_repository_set_head_detached(self.as_ptr().cast_mut(), commit.as_ptr())
        };
        status(rc)
    }

    /// As [`set_head_detached`](Self::set_head_detached) at `commit`'s commit, naming
    /// it in the reflog entry as it was looked up (its reference name or revspec).
    ///
    /// `commit` may come from another repository; its id must be of this repository's
    /// type, as `set_head_detached` requires, and name a commit present here.
    ///
    /// Wraps: git_repository_set_head_detached_from_annotated
    pub fn set_head_detached_from_annotated(
        &self,
        commit: AnnotatedCommitRef<'_, '_>,
    ) -> Result<(), RepositoryError> {
        // SAFETY: a live annotated commit, only read; the id lives inside it.
        let id = unsafe { ffi::git_annotated_commit_id(commit.as_ptr()) };
        // SAFETY: `id` addresses the commit's live id, unchanged while `commit` is
        // borrowed, which outlives this handle.
        let id = unsafe { OidRef::from_ptr(id.cast_mut()) };
        id_fits(*self, id.expect("an annotated commit always has an id"))?;
        // SAFETY: a live repository on this thread; `commit` a live annotated commit
        // with an id of the repository's type, whose id and description libgit2 reads.
        let rc = unsafe {
            ffi::git_repository_set_head_detached_from_annotated(
                self.as_ptr().cast_mut(),
                commit.as_ptr(),
            )
        };
        status(rc)
    }
}

impl RepositoryMut<'_> {
    /// Sets the identity libgit2 writes into the reflog of the repository's reference
    /// updates, overriding `user.name` / `user.email`; `None` falls back to the
    /// configuration again. Both are copied.
    ///
    /// Fails with `-1` when a copy runs out of memory, leaving the identity unchanged
    /// (libgit2 then leaks the name's copy when it is the email's that failed).
    ///
    /// Wraps: git_repository_set_ident
    pub fn set_ident(
        &mut self,
        name: Option<&CStr>,
        email: Option<&CStr>,
    ) -> Result<(), RepositoryError> {
        let name = name.map_or(ptr::null(), CStr::as_ptr);
        let email = email.map_or(ptr::null(), CStr::as_ptr);
        // SAFETY: a live repository, written through the exclusive handle: libgit2
        // frees the previous identity strings, which no handle lends past this borrow;
        // `name` and `email` NULL or NUL-terminated strings, copied.
        let rc = unsafe { ffi::git_repository_set_ident(self.as_mut_ptr(), name, email) };
        status(rc)
    }

    /// Sets the namespace the repository's references are read and written under
    /// (`refs/namespaces/<namespace>/...`, as `GIT_NAMESPACE`); `None` removes it. The
    /// name is copied.
    ///
    /// Fails with `-1` when the copy runs out of memory, which leaves the repository
    /// without a namespace.
    ///
    /// Wraps: git_repository_set_namespace
    pub fn set_namespace(&mut self, namespace: Option<&CStr>) -> Result<(), RepositoryError> {
        let namespace = namespace.map_or(ptr::null(), CStr::as_ptr);
        // SAFETY: a live repository, written through the exclusive handle: libgit2
        // frees the previous namespace string, which no handle lends past this borrow;
        // `namespace` NULL or a NUL-terminated string, copied.
        let rc = unsafe { ffi::git_repository_set_namespace(self.as_mut_ptr(), namespace) };
        status(rc)
    }

    /// Sets the repository's working directory to `workdir`, which makes it non-bare;
    /// the path is resolved (it must exist) and stored with a trailing `/`.
    ///
    /// With `update_gitlink`, also makes the change persistent: writes `workdir/.git`
    /// as a gitlink to the repository (unless `workdir` is the repository's natural
    /// parent) and records `core.worktree` (removed in that case) and
    /// `core.bare = false` in its configuration.
    ///
    /// Fails with `-1` when `workdir` does not resolve, and with libgit2's error when
    /// a write fails ([`ErrorCode::Exists`] when `workdir/.git` is not a file); the
    /// working directory then stays unchanged. A repository without a git directory
    /// is refused with `update_gitlink` ([`ErrorCode::NotFound`], GIT_ERROR_INVALID
    /// "path cannot exist in repository"), where libgit2 dereferences its missing
    /// path.
    ///
    /// Wraps: git_repository_set_workdir
    pub fn set_workdir(
        &mut self,
        workdir: &CStr,
        update_gitlink: bool,
    ) -> Result<(), RepositoryError> {
        if update_gitlink && self.as_ref().path().is_none() {
            return Err(pathless());
        }
        // SAFETY: a live repository, written through the exclusive handle: libgit2
        // frees the previous working directory string, which no handle lends past this
        // borrow; `workdir` a NUL-terminated string, read for the call. With
        // `update_gitlink`, the repository has a git directory, which libgit2 reads.
        let rc = unsafe {
            ffi::git_repository_set_workdir(
                self.as_mut_ptr(),
                workdir.as_ptr(),
                update_gitlink.into(),
            )
        };
        status(rc)
    }
}

/// The closure a [`MergeheadCallback`] calls: one id, lent for the call.
type MergeheadFn<'f> = dyn FnMut(OidRef<'_>) -> ControlFlow<NonZero<c_int>> + 'f;

/// A callback over the commits a merge in progress merges in
/// (`git_repository_mergehead_foreach_cb`): `git_repository_mergehead_foreach` calls
/// it with the id on each line of the git directory's `MERGE_HEAD`, in order, on the
/// calling thread, and stops at the first call that does not return 0, returning that
/// value.
///
/// Built over a closure with [`new`](Self::new). Its `extern "C"` trampoline lends the
/// closure each id as an [`OidRef`] valid for the call only (libgit2 parses each line
/// into a local), and returns [`ControlFlow::Continue`] as 0 and
/// [`ControlFlow::Break`]`(code)` as `code`, which a non-zero type guarantees stops
/// the walk. A panic in the closure does not cross into C: the trampoline stops the
/// walk with `GIT_EUSER` and the panic resumes once control is back in Rust, from
/// [`call`](Self::call) or from the wrapper that ran the walk.
///
/// ```
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::oid::oid::{Oid, OidType};
/// use libgit2::libgit2::repository::repository::MergeheadCallback;
///
/// # if cfg!(miri) { return; } // Miri cannot call libgit2.
/// let mut seen = Vec::new();
/// let mut collect = |id: libgit2::libgit2::oid::oid::OidRef<'_>| {
///     seen.push(id.to_hex_string().unwrap());
///     ControlFlow::Continue(())
/// };
/// let mut callback = MergeheadCallback::new(&mut collect);
/// let id = Oid::from_string(c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750", OidType::Sha1).unwrap();
/// assert_eq!(callback.call(id.as_ref()), ControlFlow::Continue(()));
/// drop(callback);
/// assert_eq!(seen, ["a65fedf39aefe402d3bb6e24df4d4f5fe4547750"]);
/// ```
///
/// The id cannot outlive the call:
///
/// ```compile_fail,E0521
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::oid::oid::OidRef;
/// use libgit2::libgit2::repository::repository::MergeheadCallback;
///
/// fn keep<'k>(kept: &mut Option<OidRef<'k>>) {
///     let mut f = |id: OidRef<'_>| {
///         *kept = Some(id);
///         ControlFlow::Continue(())
///     };
///     let _ = MergeheadCallback::new(&mut f);
/// }
/// ```
///
/// It borrows its closure exclusively and stays on its thread: libgit2 calls it
/// synchronously, and the closure need not be `Send`.
///
/// ```compile_fail,E0277
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::oid::oid::OidRef;
/// use libgit2::libgit2::repository::repository::MergeheadCallback;
///
/// fn send<T: Send>(_: T) {}
/// let mut f = |_: OidRef<'_>| ControlFlow::Continue(());
/// send(MergeheadCallback::new(&mut f));
/// ```
///
/// Wraps: git_repository_mergehead_foreach_cb
/// Handwritten
pub struct MergeheadCallback<'f> {
    f: &'f mut MergeheadFn<'f>,
    /// A panic the closure raised inside a C walk, resumed once the walk returns.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> MergeheadCallback<'f> {
    /// The callback that calls `f` with each id, until it breaks.
    pub fn new<F>(f: &'f mut F) -> Self
    where
        F: FnMut(OidRef<'_>) -> ControlFlow<NonZero<c_int>> + 'f,
    {
        Self { f, panic: None }
    }

    /// Calls the callback with `id`, through the trampoline libgit2 calls, and returns
    /// what libgit2 would read from it. A panic of the closure resumes here.
    pub fn call(&mut self, id: OidRef<'_>) -> ControlFlow<NonZero<c_int>> {
        let rc = self.walk_with(|callback, payload| {
            let callback = callback.expect("the trampoline is never NULL");
            // SAFETY: `payload` is this callback, which nothing else uses during the
            // walk, and `id` a live id lent for the call: the trampoline's contract.
            unsafe { callback(id.as_ptr(), payload) }
        });
        match NonZero::new(rc) {
            None => ControlFlow::Continue(()),
            Some(code) => ControlFlow::Break(code),
        }
    }

    /// Runs `walk` with the C callback and its payload, as the wrapper of a routine
    /// that calls the callback (`git_repository_mergehead_foreach`) hands them to it,
    /// then resumes a panic the closure raised meanwhile. The payload is this callback,
    /// which nothing else touches until `walk` returns.
    pub(crate) fn walk_with<R>(
        &mut self,
        walk: impl FnOnce(ffi::git_repository_mergehead_foreach_cb, *mut c_void) -> R,
    ) -> R {
        let result = walk(Some(mergehead_with), ptr::from_mut(self).cast());
        if let Some(panic) = self.panic.take() {
            std::panic::resume_unwind(panic);
        }
        result
    }
}

impl fmt::Debug for MergeheadCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MergeheadCallback")
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_repository_mergehead_foreach_cb` over a [`MergeheadCallback`]'s closure.
///
/// # Safety
///
/// `payload` must address a live [`MergeheadCallback`] that nothing else uses during
/// the call, and `id` be NULL or address a `git_oid` live for the call.
unsafe extern "C" fn mergehead_with(id: *const ffi::git_oid, payload: *mut c_void) -> c_int {
    // SAFETY: per this function's contract, `payload` is a live callback borrowed for
    // the call alone.
    let callback = unsafe { &mut *payload.cast::<MergeheadCallback<'_>>() };
    if callback.panic.is_some() {
        return ErrorCode::User.raw();
    }
    // SAFETY: per this function's contract, a non-NULL `id` is live for the call; the
    // handle given to the closure is valid for its call only.
    let Some(id) = (unsafe { OidRef::from_ptr(id.cast_mut()) }) else {
        // libgit2 never passes NULL; stop the walk rather than invent an id.
        return ErrorCode::User.raw();
    };
    match std::panic::catch_unwind(AssertUnwindSafe(|| (callback.f)(id))) {
        Ok(ControlFlow::Continue(())) => 0,
        Ok(ControlFlow::Break(code)) => code.get(),
        Err(panic) => {
            callback.panic = Some(panic);
            ErrorCode::User.raw()
        }
    }
}

// --- Routines: heads and lookups ---------------------------------------------------------------------

/// The failure of a routine that returned `rc`, with the thread's last error.
fn failed<T>(rc: c_int) -> Result<T, RepositoryError> {
    Err(RepositoryError::from_libgit2(rc))
}

/// A routine's yes-or-no answer: `0` or `1`, or a negative code.
fn answered(rc: c_int) -> Result<bool, RepositoryError> {
    if rc < 0 { failed(rc) } else { Ok(rc != 0) }
}

/// Finds the repository holding `start`: `start` itself, or the closest of its parent
/// directories that is a repository directory or a working directory with one in
/// `.git`. The result is the repository directory's path, with a trailing `/`
/// (`<work>/.git/`, or a bare repository's own path).
///
/// The walk stops at a filesystem boundary (a change of `st_dev`) unless `across_fs`,
/// and at any of `ceiling_dirs`, a list of absolute, symlink-free directories joined
/// by `:` (`GIT_PATH_LIST_SEPARATOR`): a ceiling directory is never searched itself,
/// but `start` always is. `None` is an empty list.
///
/// Fails with [`ErrorCode::NotFound`] when no repository is found, and with
/// libgit2's error when `start` cannot be resolved or a `.git` file cannot be read.
///
/// The result is a fresh buffer: libgit2 overwrites the `git_buf` it is given
/// without releasing what it held, which a fresh one does not hold.
///
/// ```
/// # if cfg!(miri) { return; }
/// use libgit2::libgit2::repository::repository::discover;
/// use libgit2::libgit2::util::errors::ErrorCode;
///
/// // `/` holds no repository here, nor does anything above it.
/// let error = discover(c"/", false, None).unwrap_err();
/// assert_eq!(error.code(), Some(ErrorCode::NotFound));
/// ```
///
/// Wraps: git_repository_discover
pub fn discover(
    start: &CStr,
    across_fs: bool,
    ceiling_dirs: Option<&CStr>,
) -> Result<BufVal, RepositoryError> {
    if !runtime_pinned() {
        return Err(unpinned());
    }
    let mut out = Buf::new();
    let ceiling_dirs = ceiling_dirs.map_or(ptr::null(), CStr::as_ptr);
    let buf = out.as_mut().as_mut_ptr();
    // SAFETY: `buf` addresses the fresh, empty buffer `out` holds, which libgit2 fills
    // (`git_buf_fromstr`) or leaves empty; `start` and `ceiling_dirs` are NULL (only
    // `ceiling_dirs`) or NUL-terminated strings read for the call only. The runtime is
    // pinned.
    let rc = unsafe {
        ffi::git_repository_discover(buf, start.as_ptr(), across_fs.into(), ceiling_dirs)
    };
    if rc < 0 { failed(rc) } else { Ok(out) }
}

/// Refuses to look up a worktree of `repo` when it has no common directory to look
/// it up under (an in-memory repository): libgit2 would build the worktree's path from
/// a NULL string.
fn worktrees_reachable(repo: RepositoryRef<'_>) -> Result<(), RepositoryError> {
    if repo.commondir().is_some() {
        return Ok(());
    }
    set(
        ErrorClass::Worktree,
        c"the repository has no common directory to find worktrees in",
    );
    failed(ErrorCode::NotFound.raw())
}

impl<'a> RepositoryRef<'a> {
    /// The repository's common directory, with a trailing `/`: a linked worktree's
    /// main repository directory, the repository directory itself otherwise. `None`
    /// for a repository with no paths ([`Repository::new`]).
    ///
    /// Borrowed from the repository, which sets it once, when it opens.
    ///
    /// Wraps: git_repository_commondir
    pub fn commondir(&self) -> Option<&'a CStr> {
        // SAFETY: a live repository, only read; libgit2 asserts nothing else.
        let dir = unsafe { ffi::git_repository_commondir(self.as_ptr()) };
        // SAFETY: NULL, or the repository's own copy of the path, written when it was
        // opened and freed only with it, so live for `'a`.
        (!dir.is_null()).then(|| unsafe { CStr::from_ptr(dir) })
    }

    /// Whether the repository is bare, without a working directory.
    ///
    /// Wraps: git_repository_is_bare
    pub fn is_bare(&self) -> bool {
        // SAFETY: a live repository, only read.
        unsafe { ffi::git_repository_is_bare(self.as_ptr()) != 0 }
    }

    /// Whether the repository is a linked worktree's, opened through its `.git` file.
    ///
    /// Wraps: git_repository_is_worktree
    pub fn is_worktree(&self) -> bool {
        // SAFETY: a live repository, only read.
        unsafe { ffi::git_repository_is_worktree(self.as_ptr()) != 0 }
    }

    /// The namespace the repository's reference operations are confined to
    /// (`refs/namespaces/<namespace>/`); `None` when there is none.
    ///
    /// Borrowed from the repository, which keeps it until it is replaced
    /// (`git_repository_set_namespace`, which takes the repository exclusively) or the
    /// repository is freed:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::new().unwrap();
    /// let namespace = repo.as_ref().namespace();
    /// drop(repo);
    /// let _ = namespace;
    /// ```
    ///
    /// Wraps: git_repository_get_namespace
    pub fn namespace(&self) -> Option<&'a CStr> {
        // SAFETY: a live repository, only read; the routine casts away no constness.
        let namespace = unsafe { ffi::git_repository_get_namespace(self.as_ptr().cast_mut()) };
        // SAFETY: NULL, or the repository's own copy, which only an exclusive borrow of
        // the repository replaces or frees, so live for `'a`.
        (!namespace.is_null()).then(|| unsafe { CStr::from_ptr(namespace) })
    }

    /// The name and email the repository's reflog entries are written with, as set by
    /// `git_repository_set_ident`; each `None` when unset, when the configuration's
    /// `user.name` / `user.email` are used instead.
    ///
    /// Borrowed from the repository, which keeps them until they are replaced (by an
    /// exclusive borrow) or the repository is freed.
    ///
    /// Wraps: git_repository_ident
    pub fn ident(&self) -> (Option<&'a CStr>, Option<&'a CStr>) {
        let (mut name, mut email) = (ptr::null(), ptr::null());
        // SAFETY: two writable slots and a live repository, only read.
        let rc = unsafe { ffi::git_repository_ident(&mut name, &mut email, self.as_ptr()) };
        debug_assert_eq!(rc, 0, "git_repository_ident cannot fail");
        // SAFETY: each is NULL or the repository's own copy, which only an exclusive
        // borrow of the repository replaces or frees, so live for `'a`.
        let borrowed = |s: *const c_char| (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) });
        (borrowed(name), borrowed(email))
    }

    /// Whether the repository is a shallow clone: whether its common directory holds a
    /// non-empty `shallow` file.
    ///
    /// libgit2 looks for that file by the common directory's path, which a repository
    /// without paths ([`Repository::new`]) lacks: it then looks for `shallow` in the
    /// process's current directory.
    ///
    /// Fails with libgit2's error when the file cannot be examined.
    ///
    /// Wraps: git_repository_is_shallow
    pub fn is_shallow(&self) -> Result<bool, RepositoryError> {
        // SAFETY: a live repository, only read.
        answered(unsafe { ffi::git_repository_is_shallow(self.as_ptr().cast_mut()) })
    }

    /// The path of `item` in the repository's layout: under its repository directory,
    /// its common directory or its working directory, with a trailing `/` for a
    /// directory.
    ///
    /// Fails with [`ErrorCode::NotFound`] for an item the repository cannot have: one
    /// under a working directory in a bare repository, anything in a repository without
    /// paths ([`Repository::new`]).
    ///
    /// Wraps: git_repository_item_path
    pub fn item_path(&self, item: Item) -> Result<BufVal, RepositoryError> {
        let mut out = Buf::new();
        let buf = out.as_mut().as_mut_ptr();
        // SAFETY: `buf` addresses the fresh buffer `out` holds, which libgit2 fills or
        // leaves owning nothing; a live repository, only read; and an `Item`, below
        // `GIT_REPOSITORY_ITEM__LAST`, the bound of the table libgit2 indexes unchecked.
        let rc = unsafe { ffi::git_repository_item_path(buf, self.as_ptr(), item.raw()) };
        if rc < 0 { failed(rc) } else { Ok(out) }
    }

    /// The repository's configuration: its `config` file, over the global, XDG and
    /// system ones, loaded on first use and then shared, as one more counted reference
    /// to the config the repository keeps.
    ///
    /// The config's file backends keep the repository for their `includeIf`
    /// conditions, which every reparse of a file evaluates (see
    /// [`Config`](crate::libgit2::config::config::Config)'s repository), so the
    /// reference borrows the repository, and shares the config with any other holder
    /// as long as the repository lives:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::repository::repository::Repository;
    ///
    /// let repo = Repository::open(c".").unwrap();
    /// let config = repo.as_ref().config().unwrap();
    /// drop(repo);
    /// drop(config);
    /// ```
    ///
    /// Fails with libgit2's error (code `-1`) when a configuration file cannot be read.
    ///
    /// Wraps: git_repository_config
    pub fn config(&self) -> Result<ConfigArc<'a>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and a live repository, whose config cache libgit2
        // fills through this shared handle: the repository is used on this thread only.
        let rc = unsafe { ffi::git_repository_config(&mut out, self.as_ptr().cast_mut()) };
        if rc < 0 {
            // `out` may hold the repository's own reference, which is not ours.
            return failed(rc);
        }
        // SAFETY: on success, one more counted reference to the repository's config,
        // released by `git_config_free`; the repository holds its own, and its file
        // backends read this repository, live for `'a`, which the arc carries.
        unsafe { ConfigArc::from_c(out) }.ok_or_else(|| RepositoryError::from_libgit2(-1))
    }

    /// A read-only snapshot of the repository's configuration, as
    /// [`config`](Self::config) loads it: its values do not change when the files do.
    /// A config of its own, which no repository owns and whose backends read no
    /// repository (they copied the entries), so it outlives this one.
    ///
    /// Fails with libgit2's error when a configuration file cannot be read.
    ///
    /// Wraps: git_repository_config_snapshot
    pub fn config_snapshot(&self) -> Result<ConfigBox<'static>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and a live repository, whose config cache libgit2
        // fills through this shared handle: the repository is used on this thread only.
        let rc = unsafe { ffi::git_repository_config_snapshot(&mut out, self.as_ptr().cast_mut()) };
        if rc < 0 {
            return failed(rc);
        }
        // SAFETY: on success, a fresh config (`git_config_new`) holding snapshots of the
        // repository config's backends, owned by no repository and referenced by the
        // caller alone; a snapshot backend copies the entries and keeps no repository
        // (`config_snapshot_open` ignores the one it is given), so `'static`.
        unsafe { ConfigBox::from_c(out) }.ok_or_else(|| RepositoryError::from_libgit2(-1))
    }

    /// The repository's index: its `index` file (an empty index if there is none),
    /// loaded on first use and then shared, as one more counted reference to the index
    /// the repository keeps.
    ///
    /// The index outlives the repository, which disowns it when freed.
    ///
    /// Fails with libgit2's error (code `-1`) when the index cannot be read, and for a
    /// repository without paths ([`Repository::new`]).
    ///
    /// Wraps: git_repository_index
    pub fn index(&self) -> Result<IndexArc, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and a live repository, whose index cache libgit2 fills
        // through this shared handle: the repository is used on this thread only.
        let rc = unsafe { ffi::git_repository_index(&mut out, self.as_ptr().cast_mut()) };
        if rc < 0 {
            // `out` may hold the repository's own reference, which is not ours.
            return failed(rc);
        }
        // SAFETY: on success, one more counted reference to the repository's index,
        // released by `git_index_free`; the repository holds its own.
        unsafe { IndexArc::from_c(out) }.ok_or_else(|| RepositoryError::from_libgit2(-1))
    }

    /// The id `path`'s contents would have as an object of type `kind` in this
    /// repository, after the repository's filters for the path (line endings, say),
    /// without writing the object.
    ///
    /// A relative `path` is taken under the working directory (the current directory
    /// for a bare repository). The filters are those of `as_path`: `None` uses `path`
    /// itself when it is under the working directory, and no filters otherwise; an
    /// empty `as_path` applies none.
    ///
    /// Fails with libgit2's error when the file cannot be read, a filter fails (as
    /// `core.safecrlf` can), or the path is too long.
    ///
    /// Wraps: git_repository_hashfile
    pub fn hash_file(
        &self,
        path: &CStr,
        kind: ObjectType,
        as_path: Option<&CStr>,
    ) -> Result<Oid, RepositoryError> {
        let mut id = Oid::zeroed();
        let out = id.as_mut().as_mut_ptr();
        let as_path = as_path.map_or(ptr::null(), CStr::as_ptr);
        let (repo, path, kind) = (self.as_ptr().cast_mut(), path.as_ptr(), kind.raw());
        // SAFETY: `out` addresses `id`, written only on success; `path` and `as_path`
        // are NULL (only `as_path`) or NUL-terminated strings read for the call; a live
        // repository, whose filter and attribute caches libgit2 fills through this
        // shared handle on this thread only; `kind` a named object type.
        let rc = unsafe { ffi::git_repository_hashfile(out, repo, path, kind, as_path) };
        if rc < 0 { failed(rc) } else { Ok(id) }
    }

    /// The reference `HEAD` resolves to: the branch it names, or `HEAD` itself when it
    /// is detached.
    ///
    /// Fails with [`ErrorCode::UnbornBranch`] when the branch `HEAD` names does not
    /// exist yet, with [`ErrorCode::NotFound`] when there is no `HEAD`, and otherwise
    /// with libgit2's error.
    ///
    /// The reference reads the repository it came from, so it lives no longer than
    /// this borrow of it:
    ///
    /// ```compile_fail,E0505
    /// # use libgit2::libgit2::repository::repository::RepositoryBox;
    /// fn escape(repo: RepositoryBox) {
    ///     let head = repo.as_ref().head().unwrap();
    ///     drop(repo);
    ///     drop(head);
    /// }
    /// ```
    ///
    /// Wraps: git_repository_head
    pub fn head(&self) -> Result<ReferenceBox<'a>, RepositoryError> {
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot and a live repository, whose reference database
        // libgit2 loads through this shared handle on this thread only.
        let rc = unsafe { ffi::git_repository_head(&mut out, self.as_ptr().cast_mut()) };
        if rc < 0 {
            return failed(rc);
        }
        // SAFETY: on success, a fresh reference the caller owns alone, read from (and
        // attached to) this repository's reference database, which reads the repository
        // for as long as `'a` keeps it alive.
        unsafe { ReferenceBox::from_c(out) }.ok_or_else(|| RepositoryError::from_libgit2(-1))
    }

    /// Whether `HEAD` is detached: a direct reference, to an object the repository
    /// has. A `HEAD` naming a missing object reads as not detached.
    ///
    /// Fails with libgit2's error (code `-1`) when `HEAD` or the object database cannot
    /// be read.
    ///
    /// Wraps: git_repository_head_detached
    pub fn is_head_detached(&self) -> Result<bool, RepositoryError> {
        // SAFETY: a live repository, whose object and reference databases libgit2 loads
        // through this shared handle on this thread only.
        answered(unsafe { ffi::git_repository_head_detached(self.as_ptr().cast_mut()) })
    }

    /// Whether the branch `HEAD` names does not exist yet: a new repository's, before
    /// its first commit.
    ///
    /// Fails with libgit2's error, always as code `-1`, when `HEAD` cannot be read or
    /// resolved.
    ///
    /// Wraps: git_repository_head_unborn
    pub fn is_head_unborn(&self) -> Result<bool, RepositoryError> {
        // SAFETY: a live repository, whose reference database libgit2 loads through this
        // shared handle on this thread only.
        answered(unsafe { ffi::git_repository_head_unborn(self.as_ptr().cast_mut()) })
    }

    /// Whether the repository is empty: `HEAD` names the initial branch (`master`, or
    /// `init.defaultBranch`), and there is no reference at all.
    ///
    /// Fails with [`ErrorCode::NotFound`] when there is no `HEAD`, and with libgit2's
    /// error when the configuration or the references cannot be read.
    ///
    /// Wraps: git_repository_is_empty
    pub fn is_empty(&self) -> Result<bool, RepositoryError> {
        // SAFETY: a live repository, whose reference database and config libgit2 loads
        // through this shared handle on this thread only.
        answered(unsafe { ffi::git_repository_is_empty(self.as_ptr().cast_mut()) })
    }

    /// The reference the `HEAD` of the linked worktree `name` resolves to, as
    /// [`head`](Self::head) gives it in that worktree.
    ///
    /// libgit2 reads it through a repository of its own for the worktree, which it
    /// frees before returning, leaving the reference's database pointing at the freed
    /// repository. The result is therefore a copy that reaches no repository: an
    /// [`UnattachedReference`], with the same name, target and peeled id, which may
    /// outlive this repository too.
    ///
    /// Fails with [`ErrorCode::NotFound`] when there is no worktree `name` (a
    /// repository without paths ([`Repository::new`]) has none) or the branch its
    /// `HEAD` names does not exist yet, and otherwise with libgit2's error.
    ///
    /// Wraps: git_repository_head_for_worktree
    pub fn head_for_worktree(
        &self,
        name: &CStr,
    ) -> Result<UnattachedReferenceBox, RepositoryError> {
        worktrees_reachable(*self)?;
        let mut out = ptr::null_mut();
        // SAFETY: a writable slot, a live repository with a common directory, only
        // read, and a NUL-terminated name, read for the call.
        let rc = unsafe {
            ffi::git_repository_head_for_worktree(&mut out, self.as_ptr().cast_mut(), name.as_ptr())
        };
        if rc < 0 {
            return failed(rc);
        }
        // SAFETY: on success, `out` is a fresh reference the caller owns, resolved to the
        // end so direct, whose repository libgit2 has already freed: each getter reads
        // only its fields, whose strings and ids live until it is freed below, past the
        // copy, which takes its name, target and peeled id (set only for a packed tag's
        // reference). `git_reference_free` gives back its database count, which the
        // database, not the repository, keeps.
        unsafe {
            let name = CStr::from_ptr(ffi::git_reference_name(out));
            let target = OidRef::from_ptr(ffi::git_reference_target(out).cast_mut());
            let peel = OidRef::from_ptr(ffi::git_reference_target_peel(out).cast_mut());
            let copy = target
                .ok_or_else(last)
                .and_then(|target| UnattachedReference::direct(name, target, peel))
                .map_err(|error| RepositoryError { code: -1, error });
            ffi::git_reference_free(out);
            copy
        }
    }

    /// Whether the `HEAD` of the linked worktree `name` is detached: a direct
    /// reference, whether or not the repository has its object.
    ///
    /// Fails with [`ErrorCode::NotFound`] when there is no worktree `name` (a
    /// repository without paths ([`Repository::new`]) has none) or the branch its
    /// `HEAD` names does not exist yet, and otherwise with libgit2's error.
    ///
    /// `git_repository_head_detached_for_worktree` tests the type of the reference
    /// `HEAD` resolves to, which is direct whatever `HEAD` is, so it answers 1 for an
    /// attached `HEAD` too. This reads [`head_for_worktree`](Self::head_for_worktree),
    /// which that routine calls, and answers whether the reference it resolved to is
    /// `HEAD` itself: it is exactly when `HEAD` is direct. The failures are the C
    /// routine's.
    ///
    /// Replaces: git_repository_head_detached_for_worktree
    pub fn is_head_detached_for_worktree(&self, name: &CStr) -> Result<bool, RepositoryError> {
        let head = self.head_for_worktree(name)?;
        // SAFETY: a live reference the box owns; its name lives as long as it does,
        // past the comparison.
        let resolved = unsafe { CStr::from_ptr(ffi::git_reference_name(head.as_ref().as_ptr())) };
        Ok(resolved == c"HEAD")
    }

    /// Detaches `HEAD`: points it directly at the commit the branch it names points
    /// at, recording the move in `HEAD`'s reflog. An already detached `HEAD` is
    /// rewritten in place.
    ///
    /// Fails with [`ErrorCode::UnbornBranch`] when the branch `HEAD` names does not
    /// exist yet, with libgit2's error when `HEAD` resolves to anything but a commit (a
    /// tag included, which the header says is peeled; `HEAD` is then left alone), and
    /// when `HEAD` or its log cannot be written.
    ///
    /// Wraps: git_repository_detach_head
    pub fn detach_head(&self) -> Result<(), RepositoryError> {
        // SAFETY: a live repository, whose databases libgit2 loads and whose `HEAD` it
        // rewrites through this shared handle on this thread only; no handle reads
        // `HEAD` in place.
        let rc = unsafe { ffi::git_repository_detach_head(self.as_ptr().cast_mut()) };
        if rc < 0 { failed(rc) } else { Ok(()) }
    }

    /// The parents a commit made now would have: the commit `HEAD` resolves to, then
    /// each commit `MERGE_HEAD` names, during a merge. Empty when `HEAD` names an
    /// unborn branch, or there is no `HEAD`.
    ///
    /// The array may be released after the repository (`git_commit_free` touches only
    /// the commit), but each commit keeps the repository it was parsed from
    /// (`object.repo`, uncounted), which `git_commit_owner`, `git_commit_tree`,
    /// `git_commit_parent` and the other lookups through it read: the raw commit
    /// pointers of [`CommitArrayRef::commit_ptrs`](crate::libgit2::object::commit::CommitArrayRef::commit_ptrs)
    /// may be given to those only while this repository lives, and the commit handle
    /// the `git_commit` run adds must borrow it.
    ///
    /// Fails with libgit2's error when `HEAD` cannot be resolved to a commit, or
    /// `MERGE_HEAD` cannot be read or names something other than a commit.
    ///
    /// This is `git_repository_commit_parents` composed from the routines it calls,
    /// for two of its defects:
    ///
    /// - when it fails after collecting `HEAD`'s commit (a `MERGE_HEAD` line it cannot
    ///   parse, or naming no commit), it frees the commits it collected with `free`
    ///   while the repository's object cache still holds them, which the next lookup of
    ///   one, or freeing the repository, reads: a use after free;
    /// - it puts whatever object `HEAD` resolves to in the array, as a commit: a tag,
    ///   or a tree. Here a tag is peeled to its commit, as git does, and anything else
    ///   is refused (code [`ErrorCode::InvalidSpec`]).
    ///
    /// It also leaks `HEAD`'s reference when `HEAD` names a missing object (inside
    /// `git_revparse_ext`, which drops the reference when the object lookup fails).
    /// The composition looks `HEAD` up as that routine does, and releases it.
    ///
    /// Otherwise it gives what libgit2 gives, with the same errors.
    ///
    /// Replaces: git_repository_commit_parents
    pub fn commit_parents(&self) -> Result<CommitArrayVal, RepositoryError> {
        let repo = self.as_ptr().cast_mut();
        // `HEAD` as `git_revparse_ext(.., "HEAD")` finds it: no id or description, so
        // the reference `git_reference_dwim` resolves it to, and that reference's
        // object. A `NotFound` from either (no `HEAD`, an unborn branch, a missing
        // object) means no parents.
        let mut reference = ptr::null_mut();
        // SAFETY: a writable slot, a live repository, used on this thread only, and a
        // NUL-terminated name.
        let rc = unsafe { ffi::git_reference_dwim(&mut reference, repo, c"HEAD".as_ptr()) };
        if rc == ErrorCode::NotFound.raw() {
            return Ok(CommitArray::new());
        }
        if rc < 0 {
            return failed(rc);
        }
        let mut head = ptr::null_mut();
        // SAFETY: a writable slot; `reference` is the resolved, so direct, reference the
        // lookup handed out, whose target lives until it is freed below, after the call;
        // `GIT_OBJECT_ANY` accepts any object.
        let rc = unsafe {
            ffi::git_object_lookup(
                &mut head,
                repo,
                ffi::git_reference_target(reference),
                ffi::GIT_OBJECT_ANY,
            )
        };
        // SAFETY: the reference is released once, whatever the lookup returned.
        unsafe { ffi::git_reference_free(reference) };
        if rc == ErrorCode::NotFound.raw() {
            return Ok(CommitArray::new());
        }
        if rc < 0 {
            return failed(rc);
        }
        let mut first = ptr::null_mut();
        // SAFETY: `head` is the object the lookup handed out, which the caller owns, and
        // `first` a writable slot. `GIT_OBJECT_COMMIT` is a target libgit2 accepts.
        let rc = unsafe { ffi::git_object_peel(&mut first, head, ffi::GIT_OBJECT_COMMIT) };
        // SAFETY: `head` is released once; the peel took a reference of its own.
        unsafe { ffi::git_object_free(head) };
        if rc < 0 {
            return failed(rc);
        }
        let mut parents = Parents {
            repo: *self,
            commits: vec![first.cast()],
        };
        // SAFETY: a live repository, used on this thread only; `collect_parent` is
        // `git_repository_mergehead_foreach_cb`, given a payload of the type it reads,
        // live and otherwise unused for the call.
        let rc = unsafe {
            ffi::git_repository_mergehead_foreach(
                repo,
                Some(collect_parent),
                (&raw mut parents).cast(),
            )
        };
        if rc != 0 && rc != ErrorCode::NotFound.raw() {
            return failed(rc);
        }
        let Some(buffer) = OdbDataVec::from_slice(&parents.commits) else {
            set_oom();
            return failed(-1);
        };
        let (commits, count) = buffer.into_raw_parts();
        // The array owns the commits from here on.
        parents.commits.clear();
        let array = ffi::git_commitarray {
            commits: commits.cast_const(),
            count,
        };
        // SAFETY: `commits` is a fresh buffer of libgit2's allocator holding `count`
        // commit pointers, each a reference (from `git_commit_lookup` or the peel) owned
        // by nothing else from here on.
        Ok(unsafe { CommitArray::from_raw(array) })
    }
}

/// The commits [`RepositoryRef::commit_parents`] collected; it releases each until the
/// array they move into takes them. Raw commit pointers, as [`CommitArray`] holds
/// them until the `git_commit` run wraps the commit.
struct Parents<'a> {
    repo: RepositoryRef<'a>,
    commits: Vec<*mut ffi::git_commit>,
}

impl Drop for Parents<'_> {
    fn drop(&mut self) {
        for &commit in &self.commits {
            // SAFETY: each is a commit reference this collection owns, released once.
            unsafe { ffi::git_commit_free(commit) };
        }
    }
}

/// `git_repository_mergehead_foreach_cb` collecting each `MERGE_HEAD` commit into a
/// [`Parents`], as `insert_mergehead` does: `-1` when the id names no commit or the
/// collection cannot grow.
///
/// # Safety
///
/// `oid` must be a live id, and `payload` a live [`Parents`] nothing else uses during
/// the call.
unsafe extern "C" fn collect_parent(oid: *const ffi::git_oid, payload: *mut c_void) -> c_int {
    // SAFETY: per this function's contract.
    let parents = unsafe { &mut *payload.cast::<Parents<'_>>() };
    if parents.commits.try_reserve(1).is_err() {
        set_oom();
        return -1;
    }
    let mut commit = ptr::null_mut();
    let repo = parents.repo.as_ptr().cast_mut();
    // SAFETY: a writable slot, the live repository the collection was made for, used on
    // this thread only, and a live id of the repository's type (libgit2 parsed it so).
    if unsafe { ffi::git_commit_lookup(&mut commit, repo, oid) } < 0 {
        return -1;
    }
    parents.commits.push(commit);
    0
}

// --- `FETCH_HEAD` callback -----------------------------------------------------------

/// One entry of a repository's `FETCH_HEAD`, as `git_repository_fetchhead_foreach`
/// passes it to its callback, borrowed for that call.
///
/// Handwritten
#[derive(Clone, Copy, Debug)]
pub struct FetchheadEntry<'a> {
    /// The fetched reference's name: `refs/heads/<name>` for a branch,
    /// `refs/tags/<name>` for a tag, the name as written for another quoted one;
    /// `None` for an entry naming none.
    pub ref_name: Option<&'a CStr>,
    /// The URL fetched from; `None` for an entry in the old format, an id alone.
    pub remote_url: Option<&'a CStr>,
    /// The fetched object's id, of the repository's type.
    pub id: OidRef<'a>,
    /// Whether the entry is to be merged: not marked `not-for-merge`.
    pub is_merge: bool,
}

/// What a [`FetchheadForeachCallback`] calls with each entry: `Continue` for the next
/// one, `Break` to stop.
type FetchheadFn<'f> = dyn FnMut(FetchheadEntry<'_>) -> ControlFlow<()> + 'f;

/// A `FETCH_HEAD` callback (`git_repository_fetchhead_foreach_cb`): a Rust closure
/// libgit2 calls with each entry of `FETCH_HEAD`, through a C function and a payload
/// pointing at this handle.
///
/// [`RepositoryRef::for_each_fetchhead`] hands it to libgit2, and [`call`](Self::call)
/// runs it as libgit2 does, safely. The closure is borrowed, so it keeps its state for
/// the caller to read afterwards:
///
/// ```
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::oid::oid::{Oid, OidType};
/// use libgit2::libgit2::repository::repository::{FetchheadEntry, FetchheadForeachCallback};
///
/// let mut merged = 0;
/// let mut count = |entry: FetchheadEntry<'_>| {
///     merged += usize::from(entry.is_merge);
///     ControlFlow::Continue(())
/// };
/// let mut callback = FetchheadForeachCallback::new(&mut count);
/// let id = Oid::zero(OidType::Sha1);
/// let entry = FetchheadEntry { ref_name: None, remote_url: None, id: id.as_ref(), is_merge: true };
/// assert_eq!(callback.call(entry), ControlFlow::Continue(()));
/// drop(callback);
/// assert_eq!(merged, 1);
/// ```
///
/// An entry lives for its call only:
///
/// ```compile_fail,E0521
/// use std::ops::ControlFlow;
/// use libgit2::libgit2::repository::repository::{FetchheadEntry, FetchheadForeachCallback};
///
/// let mut kept = None;
/// let mut keep = |entry: FetchheadEntry<'_>| {
///     kept = entry.ref_name;
///     ControlFlow::Continue(())
/// };
/// let _ = FetchheadForeachCallback::new(&mut keep);
/// ```
///
/// The trampoline catches a panic of the closure rather than unwind into C: the call
/// stops the iteration, and [`call`](Self::call) or
/// [`for_each_fetchhead`](RepositoryRef::for_each_fetchhead) resumes the panic once C
/// has returned.
///
/// Wraps: git_repository_fetchhead_foreach_cb
/// Handwritten
pub struct FetchheadForeachCallback<'f> {
    callback: &'f mut FetchheadFn<'f>,
    /// Whether the last call asked to stop (or panicked).
    stopped: bool,
    /// The panic the closure raised in a call, until resumed.
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl<'f> FetchheadForeachCallback<'f> {
    /// The callback calling `callback` with each entry; `Break` stops the iteration.
    pub fn new<F>(callback: &'f mut F) -> Self
    where
        F: FnMut(FetchheadEntry<'_>) -> ControlFlow<()> + 'f,
    {
        Self {
            callback,
            stopped: false,
            panic: None,
        }
    }

    /// Calls the callback with `entry` through its C function, as libgit2 does, and
    /// returns what it answered. A panic of the closure is resumed here.
    pub fn call(&mut self, entry: FetchheadEntry<'_>) -> ControlFlow<()> {
        let (callback, payload) = self.raw();
        let callback = callback.expect("the trampoline is always set");
        let ref_name = entry.ref_name.map_or(ptr::null(), CStr::as_ptr);
        let remote_url = entry.remote_url.map_or(ptr::null(), CStr::as_ptr);
        // SAFETY: the trampoline with the payload `raw` gave for it, this handle, used by
        // nothing else during the call; strings NULL or NUL-terminated and an id, all
        // live for the call, as libgit2 passes them.
        let rc = unsafe {
            callback(
                ref_name,
                remote_url,
                entry.id.as_ptr(),
                entry.is_merge.into(),
                payload,
            )
        };
        self.resume_panic();
        if rc == 0 {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    }

    /// The C function and payload `git_repository_fetchhead_foreach` is given. The
    /// function is never `None`. The payload points at this handle: it stays valid
    /// while the handle is neither moved, dropped nor used otherwise, and only for
    /// calls from the thread that holds the handle.
    ///
    /// The function returns 0 to continue, or `GIT_EUSER` to stop when the closure
    /// returned `Break` or panicked.
    pub(crate) fn raw(&mut self) -> (ffi::git_repository_fetchhead_foreach_cb, *mut c_void) {
        (Some(fetchhead_with), ptr::from_mut(self).cast())
    }

    /// Whether the last call stopped the iteration: the closure returned `Break`, or
    /// panicked. Tells the callback's `GIT_EUSER` from libgit2's own errors.
    pub fn stopped(&self) -> bool {
        self.stopped
    }

    /// Resumes a panic the closure raised during a call from C, which the trampoline
    /// caught so it did not unwind into C; does nothing when there is none.
    pub(crate) fn resume_panic(&mut self) {
        if let Some(panic) = self.panic.take() {
            panic::resume_unwind(panic);
        }
    }
}

impl fmt::Debug for FetchheadForeachCallback<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FetchheadForeachCallback")
            .field("stopped", &self.stopped)
            .field("panicked", &self.panic.is_some())
            .finish_non_exhaustive()
    }
}

/// `git_repository_fetchhead_foreach_cb` over a [`FetchheadForeachCallback`].
///
/// # Safety
///
/// `payload` must be what [`FetchheadForeachCallback::raw`] returned with this
/// function, under its terms; `ref_name` and `remote_url` NULL or NUL-terminated, and
/// `oid` NULL or an id, all live for the call.
unsafe extern "C" fn fetchhead_with(
    ref_name: *const c_char,
    remote_url: *const c_char,
    oid: *const ffi::git_oid,
    is_merge: c_uint,
    payload: *mut c_void,
) -> c_int {
    // SAFETY: per this function's contract `payload` is a live handle nothing else
    // uses during the call.
    let this = unsafe { &mut *payload.cast::<FetchheadForeachCallback<'_>>() };
    // SAFETY: NULL or an id live for the call; the handle borrows it for the call.
    let Some(id) = (unsafe { OidRef::from_ptr(oid.cast_mut()) }) else {
        // libgit2 always passes one.
        this.stopped = true;
        return ErrorCode::User.raw();
    };
    // SAFETY: each NULL or a C string live for the call, borrowed for it.
    let borrowed = |s: *const c_char| (!s.is_null()).then(|| unsafe { CStr::from_ptr(s) });
    let entry = FetchheadEntry {
        ref_name: borrowed(ref_name),
        remote_url: borrowed(remote_url),
        id,
        is_merge: is_merge != 0,
    };
    let callback = &mut *this.callback;
    let answer = panic::catch_unwind(AssertUnwindSafe(|| callback(entry)));
    this.stopped = !matches!(answer, Ok(ControlFlow::Continue(())));
    match answer {
        Ok(ControlFlow::Continue(())) => 0,
        Ok(ControlFlow::Break(())) => ErrorCode::User.raw(),
        Err(panic) => {
            this.panic = Some(panic);
            ErrorCode::User.raw()
        }
    }
}

impl RepositoryRef<'_> {
    /// Calls `callback` with each entry of the repository's `FETCH_HEAD`, in file
    /// order, until its closure returns `Break`: `Ok(Continue(()))` when every entry
    /// was seen, `Ok(Break(()))` when the closure stopped the walk. A panic of the
    /// closure stops the walk too, and is resumed here once libgit2 has returned.
    ///
    /// Each line is parsed before its call, so the entries before a malformed line
    /// are seen and the walk then fails with `GIT_ERROR_FETCHHEAD` and the line's
    /// number (an empty line, a bad id, a bad `not-for-merge` marker or description,
    /// a last line without its newline); ids are read as the repository's type. Fails
    /// with [`ErrorCode::NotFound`] when there is no `FETCH_HEAD`, and for a repository
    /// without paths, whose `FETCH_HEAD` libgit2 would read from the process's working
    /// directory.
    ///
    /// libgit2 reads the file and calls back synchronously, on this thread, so the
    /// closure may use this repository's shared handles meanwhile, but not its
    /// exclusive one:
    ///
    /// ```compile_fail,E0502
    /// use std::ops::ControlFlow;
    /// use libgit2::libgit2::repository::repository::{
    ///     FetchheadEntry, FetchheadForeachCallback, RepositoryBox,
    /// };
    ///
    /// fn walk(mut repo: RepositoryBox) {
    ///     let mut rename = |_: FetchheadEntry<'_>| {
    ///         let _ = repo.as_mut().set_namespace(None);
    ///         ControlFlow::Continue(())
    ///     };
    ///     let mut callback = FetchheadForeachCallback::new(&mut rename);
    ///     let _ = repo.as_ref().for_each_fetchhead(&mut callback);
    /// }
    /// ```
    ///
    /// When the closure stops the walk, libgit2 leaves
    /// "`git_repository_fetchhead_foreach` callback returned -7" as the thread's error
    /// unless one is set already (by a libgit2 call the closure made, say).
    ///
    /// Wraps: git_repository_fetchhead_foreach
    pub fn for_each_fetchhead(
        &self,
        callback: &mut FetchheadForeachCallback<'_>,
    ) -> Result<ControlFlow<()>, RepositoryError> {
        if !runtime_pinned() {
            return Err(unpinned());
        }
        if self.path().is_none() {
            return Err(pathless());
        }
        callback.stopped = false;
        let repo = self.as_ptr().cast_mut();
        let (f, payload) = callback.raw();
        // SAFETY: a live repository with a git directory, used on this thread, which
        // the routine only reads (its `gitdir` and object id type); the callback's own
        // trampoline and payload, which nothing else touches until the call returns
        // (`callback` is exclusively borrowed). The runtime is pinned.
        let rc = unsafe { ffi::git_repository_fetchhead_foreach(repo, f, payload) };
        callback.resume_panic();
        match rc {
            0 => Ok(ControlFlow::Continue(())),
            ffi::GIT_EUSER if callback.stopped => Ok(ControlFlow::Break(())),
            rc => Err(RepositoryError::from_libgit2(rc)),
        }
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

        /// When libgit2 cannot be initialised, every constructor (and `discover`) fails
        /// without calling it, with the error `last` synthesises, and the policy builds
        /// nothing.
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
                        Repository::init_ext(c".", InitOptions::new().as_mut()).unwrap_err(),
                        Repository::new_ext(NewOptions::new().as_ref()).unwrap_err(),
                        Repository::open_ext(Some(c"."), OpenFlags::empty(), None).unwrap_err(),
                        Repository::exists_ext(Some(c"."), OpenFlags::empty(), None).unwrap_err(),
                        discover(c".", true, None).unwrap_err(),
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

    mod routines {
        use super::super::*;
        use std::ffi::CString;
        use std::fs;
        use std::os::unix::ffi::OsStrExt;
        use std::path::{Path, PathBuf};
        use std::process::Command;

        use crate::libgit2::odb::odb::Odb;

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// Runs `body` alone in a child process, which may change its working directory.
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

        fn scratch(tag: &str) -> PathBuf {
            let path = std::env::temp_dir().join(format!(
                "crustify-repository-routines-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            path
        }

        fn c_path(path: &Path) -> CString {
            CString::new(path.as_os_str().as_bytes()).unwrap()
        }

        /// Lays the state files `git_repository_state_cleanup` removes out in `dir`.
        fn lay_state(dir: &Path) {
            for file in ["MERGE_HEAD", "MERGE_MODE", "MERGE_MSG", "BISECT_LOG"] {
                fs::write(dir.join(file), "x\n").unwrap();
            }
            for sub in ["rebase-merge", "rebase-apply", "sequencer"] {
                fs::create_dir_all(dir.join(sub)).unwrap();
                fs::write(dir.join(sub).join("todo"), "x\n").unwrap();
            }
        }

        fn state_left(dir: &Path) -> Vec<&'static str> {
            [
                "MERGE_HEAD",
                "MERGE_MODE",
                "MERGE_MSG",
                "BISECT_LOG",
                "rebase-merge",
                "rebase-apply",
                "sequencer",
            ]
            .into_iter()
            .filter(|f| dir.join(f).exists())
            .collect()
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn cleanup_state_refuses_a_repository_without_paths() {
            isolated(
                concat!(
                    module_path!(),
                    "::cleanup_state_refuses_a_repository_without_paths"
                ),
                || {
                    // libgit2 resolves the state files against the NULL git directory of a
                    // repository without paths, so against the working directory.
                    let cwd = scratch("cwd");
                    std::env::set_current_dir(&cwd).unwrap();
                    lay_state(&cwd);
                    let c_repo = Repository::new().unwrap();
                    // SAFETY: the box's live repository, on this thread.
                    let rc = unsafe { ffi::git_repository_state_cleanup(c_repo.as_c_ptr()) };
                    assert_eq!(rc, 0);
                    assert_eq!(state_left(&cwd), Vec::<&str>::new(), "C removed them");

                    lay_state(&cwd);
                    let odb = Odb::new().unwrap();
                    for mut repo in [
                        Repository::new().unwrap(),
                        Repository::wrap_odb(odb).unwrap(),
                    ] {
                        let e = repo.as_mut().cleanup_state().unwrap_err();
                        assert_eq!(e.code(), Some(ErrorCode::NotFound));
                        assert_eq!(e.class(), Some(ErrorClass::Invalid));
                        assert_eq!(e.message(), c"path cannot exist in repository");
                    }
                    assert_eq!(state_left(&cwd).len(), 7, "Rust left them");
                    std::env::set_current_dir(std::env::temp_dir()).unwrap();
                    fs::remove_dir_all(&cwd).unwrap();
                },
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn cleanup_state_clears_a_stale_error() {
            let dir = scratch("stale");
            let mut repo = Repository::init(&c_path(&dir), true).unwrap();
            lay_state(&dir);
            crate::libgit2::util::sys::errors::set(ErrorClass::Odb, c"stale");
            repo.as_mut().cleanup_state().unwrap();
            assert_eq!(last().class(), Some(ErrorClass::None));
            assert!(state_left(&dir).is_empty());
            drop(repo);
            fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn wrap_odb_releases_the_odb_with_the_repository() {
            let odb = Odb::with_oid_type(OidType::Sha256).unwrap();
            let repo = Repository::wrap_odb(odb).unwrap();
            // SAFETY: the box's live repository, only read.
            let kind = unsafe { ffi::git_repository_oid_type(repo.as_c_ptr()) };
            assert_eq!(kind, OidType::Sha256.raw());
            assert_eq!(repo.as_ref().workdir(), None);
            drop(repo);
            // LSan sees the odb freed with its last reference, the repository's.
        }
    }

    mod heads_and_parts {
        use super::super::*;

        use std::ffi::CString;
        use std::fs;
        use std::os::unix::ffi::OsStrExt;
        use std::path::{Path, PathBuf};
        use std::process::Command;

        use crate::libgit2::oid::oid::Oid;
        use crate::libgit2::repository::worktree::WorktreeBox;

        /// A scratch directory of its own under the temp dir, removed when dropped.
        struct Scratch(PathBuf);

        impl Scratch {
            fn new(tag: &str) -> Self {
                let path = std::env::temp_dir().join(format!(
                    "crustify-repository-routines-unit-{}-{tag}",
                    std::process::id()
                ));
                let _ = fs::remove_dir_all(&path);
                fs::create_dir_all(&path).unwrap();
                Self(path.canonicalize().unwrap())
            }

            fn c(&self, rel: &str) -> CString {
                CString::new(self.0.join(rel).as_os_str().as_bytes()).unwrap()
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

        /// Runs `body` alone in a child process whose current directory is `dir`.
        fn in_dir(test: &str, dir: &Path, body: fn()) {
            let name = test.split_once("::").map_or(test, |(_, rest)| rest);
            if std::env::var(CHILD).as_deref() == Ok(name) {
                body();
                return;
            }
            let out = Command::new(std::env::current_exe().unwrap())
                .args([name, "--exact", "--test-threads=1", "--nocapture"])
                .env(CHILD, name)
                .current_dir(dir)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert!(
                out.status.success() && stdout.contains("1 passed"),
                "child `{name}` failed:\n{stdout}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }

        const IDS: [&CStr; 3] = [
            c"a65fedf39aefe402d3bb6e24df4d4f5fe4547750",
            c"be3563ae3f795b2b4353bcce3a527ad0a4f7f644",
            c"c47800c7266a2be04c571c04d5a6614691ea99bd",
        ];

        /// A bare repository whose `MERGE_HEAD` lists [`IDS`].
        fn merging(scratch: &Scratch) -> RepositoryBox {
            let repo = Repository::init(&scratch.c("repo"), true).unwrap();
            let lines: String = IDS
                .iter()
                .map(|id| format!("{}\n", id.to_str().unwrap()))
                .collect();
            fs::write(scratch.0.join("repo/MERGE_HEAD"), lines).unwrap();
            repo
        }

        /// What a raw C callback records: the ids it saw, and the call to stop at.
        struct Recorder {
            seen: Vec<String>,
            stop_at: usize,
            code: c_int,
        }

        unsafe extern "C" fn record(id: *const ffi::git_oid, payload: *mut c_void) -> c_int {
            // SAFETY: the tests pass a live `Recorder` as the payload and libgit2 a live
            // id; both are used for the call only.
            let (recorder, id) = unsafe {
                (
                    &mut *payload.cast::<Recorder>(),
                    OidRef::from_ptr(id.cast_mut()).unwrap(),
                )
            };
            recorder.seen.push(id.to_hex_string().unwrap());
            if recorder.seen.len() == recorder.stop_at {
                recorder.code
            } else {
                0
            }
        }

        /// `git_repository_mergehead_foreach` over `callback` / `payload`.
        fn walk(
            repo: &RepositoryBox,
            callback: ffi::git_repository_mergehead_foreach_cb,
            payload: *mut c_void,
        ) -> c_int {
            // SAFETY: a live repository; `callback` and `payload` are a pair the callee
            // may call on this thread for the duration of the call.
            unsafe { ffi::git_repository_mergehead_foreach(repo.as_c_ptr(), callback, payload) }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn mergehead_callback_sees_and_stops_as_a_c_callback() {
            let scratch = Scratch::new("mergehead-walk");
            let repo = merging(&scratch);
            for (stop_at, code) in [(usize::MAX, 0), (2, 7), (1, -30), (3, 1)] {
                let mut c = Recorder {
                    seen: Vec::new(),
                    stop_at,
                    code,
                };
                let c_rc = walk(&repo, Some(record), ptr::from_mut(&mut c).cast());

                let mut seen = Vec::new();
                let mut f = |id: OidRef<'_>| {
                    seen.push(id.to_hex_string().unwrap());
                    match NonZero::new(code) {
                        Some(code) if seen.len() == stop_at => ControlFlow::Break(code),
                        _ => ControlFlow::Continue(()),
                    }
                };
                let mut callback = MergeheadCallback::new(&mut f);
                let rust_rc = callback.walk_with(|cb, payload| walk(&repo, cb, payload));
                drop(callback);
                assert_eq!((rust_rc, &seen), (c_rc, &c.seen), "stop at {stop_at}");
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn mergehead_callback_resumes_a_panic_after_the_walk() {
            let scratch = Scratch::new("mergehead-panic");
            let repo = merging(&scratch);
            let mut calls = 0;
            let mut f = |_: OidRef<'_>| -> ControlFlow<NonZero<c_int>> {
                calls += 1;
                panic!("boom");
            };
            let mut callback = MergeheadCallback::new(&mut f);
            let mut rc = 0;
            let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
                callback.walk_with(|cb, payload| rc = walk(&repo, cb, payload))
            }));
            // The panic stopped the walk at the first id, without unwinding through C,
            // and resumed once it returned.
            assert_eq!(rc, ErrorCode::User.raw());
            assert_eq!(*panic.unwrap_err().downcast::<&str>().unwrap(), "boom");
            // Resumed once: the slot is empty again.
            assert!(format!("{callback:?}").contains("panicked: false"));
            assert_eq!(callback.walk_with(|_, _| 3), 3);
            drop(callback);
            assert_eq!(calls, 1);
        }

        #[test]
        fn mergehead_trampoline_skips_a_null_id_and_a_pending_panic() {
            let mut calls = 0;
            let mut f = |_: OidRef<'_>| -> ControlFlow<NonZero<c_int>> {
                calls += 1;
                panic!("first");
            };
            let mut callback = MergeheadCallback::new(&mut f);
            let id = Oid::zero(OidType::Sha1);
            let mut codes = Vec::new();
            let panic = std::panic::catch_unwind(AssertUnwindSafe(|| {
                callback.walk_with(|cb, payload| {
                    let cb = cb.unwrap();
                    // SAFETY: the payload is the live callback, used by nothing else
                    // during the walk; NULL, then a live id, are the ids under test.
                    unsafe {
                        codes.push(cb(ptr::null(), payload));
                        codes.push(cb(id.as_ref().as_ptr(), payload));
                        // A walk that ignored the stop calls again: the pending panic
                        // keeps the closure from running.
                        codes.push(cb(id.as_ref().as_ptr(), payload));
                    }
                })
            }));
            assert_eq!(*panic.unwrap_err().downcast::<&str>().unwrap(), "first");
            drop(callback);
            assert_eq!(codes, [ErrorCode::User.raw(); 3]);
            assert_eq!(calls, 1);
        }

        /// The thread's last error, as C reads it: class and message.
        fn c_last() -> (c_int, String) {
            // SAFETY: the runtime is pinned by the repository the callers built; the
            // error is copied before the next libgit2 call.
            let e = unsafe { ffi::git_error_last() };
            // SAFETY: `git_error_last` returns a live error with a message.
            unsafe {
                (
                    (*e).klass,
                    CStr::from_ptr((*e).message).to_string_lossy().into(),
                )
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn remove_message_reports_the_os_error_c_leaves_unset() {
            let scratch = Scratch::new("remove-message");
            let repo = Repository::init(&scratch.c("repo"), true).unwrap();
            clear();
            // SAFETY: a live repository; the absent `MERGE_MSG` is the case under test.
            let rc = unsafe { ffi::git_repository_message_remove(repo.as_c_ptr()) };
            assert_eq!(
                (rc, c_last()),
                (-1, (ffi::GIT_ERROR_NONE as c_int, "no error".into()))
            );

            let error = repo.as_ref().remove_message().unwrap_err();
            assert_eq!(error.raw_code(), -1);
            assert_eq!(error.class(), Some(ErrorClass::Os));
            assert_eq!(
                error.message().to_str(),
                Ok("could not remove message file: No such file or directory")
            );
            // An error libgit2 does set is kept: a directory in the way.
            fs::create_dir(scratch.0.join("repo/MERGE_MSG")).unwrap();
            let error = repo.as_ref().remove_message().unwrap_err();
            assert_eq!(error.raw_code(), -1);
            assert_eq!(error.class(), Some(ErrorClass::Os));
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn open_from_worktree_reports_the_error_c_leaves_unset() {
            let scratch = Scratch::new("worktree-gitdir");
            let main = Repository::init(&scratch.c("main"), false).unwrap();
            let admin = scratch.0.join("main/.git/worktrees/wt");
            fs::create_dir_all(&admin).unwrap();
            fs::create_dir_all(scratch.0.join("wt")).unwrap();
            fs::write(admin.join("commondir"), "../..\n").unwrap();
            fs::write(admin.join("HEAD"), "ref: refs/heads/wt\n").unwrap();
            // The recorded gitlink does not end in `.git`.
            let gitlink = scratch.0.join("wt/link");
            fs::write(admin.join("gitdir"), format!("{}\n", gitlink.display())).unwrap();
            fs::write(&gitlink, format!("gitdir: {}\n", admin.display())).unwrap();

            let mut wt = ptr::null_mut();
            // SAFETY: a live slot, repository and name.
            let rc = unsafe { ffi::git_worktree_lookup(&mut wt, main.as_c_ptr(), c"wt".as_ptr()) };
            assert_eq!(rc, 0, "{:?}", c_last());
            // SAFETY: the fresh worktree, owned here.
            let wt = unsafe { WorktreeBox::from_c(wt) }.unwrap();

            clear();
            let mut out = ptr::null_mut();
            // SAFETY: a live slot and worktree.
            let rc = unsafe { ffi::git_repository_open_from_worktree(&mut out, wt.as_c_ptr()) };
            assert!(out.is_null());
            assert_eq!(
                (rc, c_last()),
                (-1, (ffi::GIT_ERROR_NONE as c_int, "no error".into()))
            );

            let error = Repository::open_from_worktree(wt.as_ref()).unwrap_err();
            assert_eq!(error.raw_code(), -1);
            assert_eq!(error.class(), Some(ErrorClass::Worktree));
            assert_eq!(
                error.message().to_str(),
                Ok("worktree gitdir does not end in '.git'")
            );
        }

        /// Where libgit2 resolves a pathless repository's git directory against the
        /// current directory, the wrappers refuse: run in a child whose current
        /// directory holds a `MERGE_MSG`.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn pathless_routines_refuse_where_c_uses_the_current_directory() {
            let scratch = Scratch::new("pathless");
            fs::write(scratch.0.join("MERGE_MSG"), "from the cwd\n").unwrap();
            in_dir(
                concat!(
                    module_path!(),
                    "::pathless_routines_refuse_where_c_uses_the_current_directory"
                ),
                &scratch.0,
                || {
                    let mut repo = Repository::new().unwrap();
                    assert!(repo.as_ref().path().is_none());

                    // C reads and removes the current directory's `MERGE_MSG`.
                    let mut buf = Buf::new();
                    // SAFETY: a live empty buffer and repository.
                    let rc = unsafe {
                        ffi::git_repository_message(buf.as_mut().as_mut_ptr(), repo.as_c_ptr())
                    };
                    assert_eq!((rc, buf.as_ref().as_bytes()), (0, &b"from the cwd\n"[..]));
                    let refused = |e: RepositoryError| {
                        assert_eq!(e.code(), Some(ErrorCode::NotFound));
                        assert_eq!(e.class(), Some(ErrorClass::Invalid));
                        assert_eq!(e.message().to_str(), Ok("path cannot exist in repository"));
                    };
                    refused(repo.as_ref().message().unwrap_err());
                    refused(repo.as_ref().remove_message().unwrap_err());
                    assert!(Path::new("MERGE_MSG").exists());
                    // SAFETY: a live repository.
                    let rc = unsafe { ffi::git_repository_message_remove(repo.as_c_ptr()) };
                    assert_eq!(rc, 0);
                    assert!(!Path::new("MERGE_MSG").exists());

                    // C writes a `config` in the current directory.
                    refused(repo.as_mut().reinit_filesystem(false).unwrap_err());
                    assert!(!Path::new("config").exists());
                    // SAFETY: a live repository.
                    let rc = unsafe { ffi::git_repository_reinit_filesystem(repo.as_c_ptr(), 0) };
                    assert_eq!(rc, 0);
                    assert!(Path::new("config").exists());

                    // C dereferences the missing path (SIGSEGV): never called.
                    refused(repo.as_mut().set_workdir(c".", true).unwrap_err());
                    assert!(!Path::new(".git").exists());
                },
            );
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn pathless_state_is_refused_as_a_release_build_refuses_it() {
            let repo = Repository::new().unwrap();
            let error = repo.as_ref().state().unwrap_err();
            assert_eq!(error.raw_code(), -1);
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
            assert_eq!(error.message().to_str(), Ok("invalid argument: 'string'"));
            // A debug build (the coverage variant) aborts on the assertion instead.
            if std::env::var("CRUSTIFY_BUILD_VARIANT").as_deref() != Ok("coverage") {
                // SAFETY: a live repository.
                assert_eq!(unsafe { ffi::git_repository_state(repo.as_c_ptr()) }, -1);
                assert_eq!(
                    c_last(),
                    (
                        ffi::GIT_ERROR_INVALID as c_int,
                        "invalid argument: 'string'".into()
                    )
                );
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn a_null_start_is_refused_without_from_env() {
            for flags in [OpenFlags::empty(), OpenFlag::NoSearch | OpenFlag::Bare] {
                let open = Repository::open_ext(None, flags, None).unwrap_err();
                let exists = Repository::exists_ext(None, flags, None).unwrap_err();
                for error in [open, exists] {
                    assert_eq!(error.raw_code(), -1);
                    assert_eq!(error.class(), Some(ErrorClass::Invalid));
                    assert_eq!(error.message().to_str(), Ok("invalid argument: 'path'"));
                }
            }
        }

        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn status_and_quiet_failures() {
            assert!(status(0).is_ok());
            assert!(status(3).is_ok());
            assert_eq!(status(-3).unwrap_err().code(), Some(ErrorCode::NotFound));
            let _pinned = Repository::new().unwrap();
            clear();
            let quiet = failed_quietly(-1, ErrorClass::Os, c"quiet");
            assert_eq!(quiet.class(), Some(ErrorClass::Os));
            assert_eq!(quiet.message().to_str(), Ok("quiet: "));
            set(ErrorClass::Odb, c"loud");
            let loud = failed_quietly(-1, ErrorClass::Os, c"quiet");
            assert_eq!(loud.class(), Some(ErrorClass::Odb));
            assert_eq!(loud.message().to_str(), Ok("loud"));
        }
    }

    mod head_and_lookups {
        use super::super::*;

        use std::ffi::CString;
        use std::fs;
        use std::os::unix::ffi::OsStrExt;
        use std::path::{Path, PathBuf};

        const RESOURCES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../tests/resources");

        /// `testrepo.git`'s `HEAD` commit, a tag object pointing at a commit, and a blob.
        const HEAD_COMMIT: &str = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750";
        const TAG: &str = "b25fa35b38051e4ae45d4222e795f9df2e43f1d1";
        const BLOB: &str = "1385f264afb75a56a5bec74243be9b367ba4ca08";

        fn copy_tree(from: &Path, to: &Path) {
            fs::create_dir_all(to).unwrap();
            for entry in fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let target = to.join(entry.file_name());
                let kind = entry.file_type().unwrap();
                if kind.is_symlink() {
                    std::os::unix::fs::symlink(fs::read_link(entry.path()).unwrap(), &target)
                        .unwrap();
                } else if kind.is_dir() {
                    copy_tree(&entry.path(), &target);
                } else {
                    fs::copy(entry.path(), &target).unwrap();
                }
            }
        }

        /// Copies of libgit2's test repositories under a scratch directory, removed on
        /// drop; `.gitted` renamed `.git`.
        struct Sandbox(PathBuf);

        impl Sandbox {
            fn new(tag: &str, fixtures: &[(&str, &str)]) -> Self {
                let root = std::env::temp_dir().join(format!(
                    "crustify-repository-routines-unit-{}-{tag}",
                    std::process::id()
                ));
                let _ = fs::remove_dir_all(&root);
                for &(name, to) in fixtures {
                    let to = root.join(to);
                    copy_tree(&Path::new(RESOURCES).join(name), &to);
                    if to.join(".gitted").exists() {
                        fs::rename(to.join(".gitted"), to.join(".git")).unwrap();
                    }
                }
                Self(root)
            }

            fn bare(tag: &str) -> Self {
                Self::new(tag, &[("testrepo.git", "bare")])
            }

            fn c_path(&self, rel: &str) -> CString {
                CString::new(self.0.join(rel).as_os_str().as_bytes()).unwrap()
            }

            fn write(&self, rel: &str, contents: &str) {
                fs::write(self.0.join(rel), contents).unwrap();
            }

            fn open(&self, rel: &str) -> RepositoryBox {
                Repository::open(&self.c_path(rel)).unwrap()
            }
        }

        impl Drop for Sandbox {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        /// A live object's id, in hex.
        fn hex(object: *mut ffi::git_commit) -> String {
            // SAFETY: the tests pass objects their arrays keep alive; the id is copied.
            let id = unsafe { OidRef::from_ptr(ffi::git_commit_id(object).cast_mut()) };
            id.unwrap().to_hex_string().unwrap()
        }

        /// The object types of an array's entries, as libgit2 tags them.
        fn types(array: &CommitArrayVal) -> Vec<ffi::git_object_t> {
            array
                .as_ref()
                .commit_ptrs()
                .elems()
                // SAFETY: each is a live object the array owns a reference to.
                .map(|object| unsafe { ffi::git_object_type(object.cast()) })
                .collect()
        }

        /// `git_repository_commit_parents` itself, adopted; only for repositories it is
        /// defined on (no failure after collecting `HEAD`).
        fn c_commit_parents(repo: &RepositoryBox) -> CommitArrayVal {
            let mut raw = ffi::git_commitarray::default();
            let repo = repo.as_c_ptr();
            // SAFETY: an empty array and a live repository.
            let rc = unsafe { ffi::git_repository_commit_parents(&mut raw, repo) };
            assert_eq!(rc, 0);
            // SAFETY: what libgit2 filled: a buffer of its allocator holding references
            // the array owns; `git_commit_free` releases any object type alike.
            unsafe { CommitArray::from_raw(raw) }
        }

        /// Corrects C: with a `MERGE_HEAD` it cannot use after `HEAD`'s commit was
        /// collected, `git_repository_commit_parents` frees that commit with `free`
        /// while the object cache holds it (a use after free on its next lookup, or
        /// when the repository is freed). The wrapper reports the failure, and the
        /// commit stays cached and usable: ASan watches the lookups and the free.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn a_bad_merge_head_fails_without_freeing_cached_commits() {
            let sandbox = Sandbox::bare("bad-merge-head");
            let repo = sandbox.open("bare");
            for (merge_head, message) in [
                ("short\n", Some("unable to parse OID - invalid length")),
                ("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\n", None),
                (&*format!("{BLOB}\n"), None),
                ("0123456789012345678901234567890123456789\n", None),
                (
                    &*format!("{HEAD_COMMIT}\n{HEAD_COMMIT}"),
                    Some("no EOL at line 2"),
                ),
            ] {
                sandbox.write("bare/MERGE_HEAD", merge_head);
                let error = repo.as_ref().commit_parents().unwrap_err();
                assert!(error.raw_code() < 0, "{merge_head:?}");
                if let Some(message) = message {
                    assert_eq!(error.message().to_str().unwrap(), message);
                }
                // HEAD's commit, cached by the failed call, is looked up again.
                fs::remove_file(sandbox.0.join("bare/MERGE_HEAD")).unwrap();
                let parents = repo.as_ref().commit_parents().unwrap();
                let ids: Vec<_> = parents.as_ref().commit_ptrs().elems().map(hex).collect();
                assert_eq!(ids, [HEAD_COMMIT]);
            }
            drop(repo);
        }

        /// `MERGE_HEAD`'s commits follow `HEAD`'s, in order, repeats included, as C
        /// collects them.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn merge_heads_follow_head_in_order() {
            let sandbox = Sandbox::bare("merge-heads");
            let repo = sandbox.open("bare");
            sandbox.write(
                "bare/MERGE_HEAD",
                &format!("{HEAD_COMMIT}\n{HEAD_COMMIT}\n"),
            );
            let ids: Vec<_> = repo
                .as_ref()
                .commit_parents()
                .unwrap()
                .as_ref()
                .commit_ptrs()
                .elems()
                .map(hex)
                .collect();
            assert_eq!(ids, [HEAD_COMMIT; 3]);
            let c: Vec<_> = c_commit_parents(&repo)
                .as_ref()
                .commit_ptrs()
                .elems()
                .map(hex)
                .collect();
            assert_eq!(ids, c);
        }

        /// Corrects C: with `HEAD` naming a missing object, `git_revparse_ext` (which
        /// `git_repository_commit_parents` calls) leaks `HEAD`'s reference. The wrapper
        /// gives no parents, as C does, and LSan sees nothing left over.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn a_head_at_a_missing_object_gives_no_parents_without_leaking() {
            let sandbox = Sandbox::bare("missing-head");
            let repo = sandbox.open("bare");
            sandbox.write("bare/HEAD", "0123456789012345678901234567890123456789\n");
            for _ in 0..3 {
                assert!(repo.as_ref().commit_parents().unwrap().as_ref().is_empty());
            }
            sandbox.write("bare/HEAD", "ref: refs/heads/master\n");
            sandbox.write(
                "bare/refs/heads/master",
                "0123456789012345678901234567890123456789\n",
            );
            assert!(repo.as_ref().commit_parents().unwrap().as_ref().is_empty());
        }

        /// Corrects C: a `HEAD` detached at a tag puts the tag object in C's array of
        /// commits; the wrapper peels it to the tagged commit, as git does.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn a_tag_head_is_peeled_where_c_returns_the_tag() {
            let sandbox = Sandbox::bare("tag-head");
            let repo = sandbox.open("bare");
            sandbox.write("bare/HEAD", &format!("{TAG}\n"));
            let c = c_commit_parents(&repo);
            assert_eq!(types(&c), [ffi::GIT_OBJECT_TAG]);
            let rust = repo.as_ref().commit_parents().unwrap();
            assert_eq!(types(&rust), [ffi::GIT_OBJECT_COMMIT]);
            let mut peeled = ptr::null_mut();
            let tag = c.as_ref().commit_ptrs().elems().next().unwrap();
            let commit = ffi::GIT_OBJECT_COMMIT;
            // SAFETY: a live tag object and a writable slot; the peel is freed below.
            let rc = unsafe { ffi::git_object_peel(&mut peeled, tag.cast(), commit) };
            assert_eq!(rc, 0);
            assert_eq!(
                rust.as_ref()
                    .commit_ptrs()
                    .elems()
                    .map(hex)
                    .collect::<Vec<_>>(),
                [hex(peeled.cast())]
            );
            // SAFETY: the peel's own reference, released once.
            unsafe { ffi::git_object_free(peeled) };
        }

        /// Corrects C: a `HEAD` at a blob puts the blob in C's array of commits; the
        /// wrapper refuses it, as the peel to a commit fails.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn a_blob_head_is_refused_where_c_returns_the_blob() {
            let sandbox = Sandbox::bare("blob-head");
            let repo = sandbox.open("bare");
            sandbox.write("bare/HEAD", &format!("{BLOB}\n"));
            assert_eq!(types(&c_commit_parents(&repo)), [ffi::GIT_OBJECT_BLOB]);
            let error = repo.as_ref().commit_parents().unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::InvalidSpec));
            assert_eq!(error.class(), Some(ErrorClass::Object));
        }

        /// Corrects C: `git_repository_head_detached_for_worktree` answers 1 for an
        /// attached worktree `HEAD`, as it tests the reference `HEAD` resolves to.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn an_attached_worktree_head_is_not_detached() {
            let sandbox = Sandbox::new(
                "worktree-detached",
                &[
                    ("testrepo", "testrepo"),
                    ("testrepo-worktree", "testrepo-worktree"),
                ],
            );
            let repo = sandbox.open("testrepo");
            let name = c"testrepo-worktree";
            // SAFETY: a live repository and a NUL-terminated name.
            let c = unsafe {
                ffi::git_repository_head_detached_for_worktree(repo.as_c_ptr(), name.as_ptr())
            };
            assert_eq!(c, 1);
            assert!(!repo.as_ref().is_head_detached_for_worktree(name).unwrap());
            sandbox.write(
                "testrepo/.git/worktrees/testrepo-worktree/HEAD",
                &format!("{HEAD_COMMIT}\n"),
            );
            assert!(repo.as_ref().is_head_detached_for_worktree(name).unwrap());
        }

        /// Corrects C: in a repository without a common directory, libgit2's worktree
        /// lookup takes `strlen` of NULL. The wrapper refuses first.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn worktrees_of_a_pathless_repository_are_not_found() {
            let repo = Repository::new().unwrap();
            let error = repo.as_ref().head_for_worktree(c"any").unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::NotFound));
            assert_eq!(error.class(), Some(ErrorClass::Worktree));
            assert_eq!(
                error.message().to_str().unwrap(),
                "the repository has no common directory to find worktrees in"
            );
            let error = repo
                .as_ref()
                .is_head_detached_for_worktree(c"any")
                .unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::NotFound));
        }

        /// The copy of a worktree's `HEAD` carries the original's fields (not its
        /// database: `git_reference_owner` dereferences the NULL one of a copy).
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn a_worktree_head_is_copied_with_its_fields() {
            let sandbox = Sandbox::new(
                "worktree-copy",
                &[
                    ("testrepo", "testrepo"),
                    ("testrepo-worktree", "testrepo-worktree"),
                ],
            );
            let repo = sandbox.open("testrepo");
            let head = repo
                .as_ref()
                .head_for_worktree(c"testrepo-worktree")
                .unwrap();
            let raw = head.as_ref().as_ptr();
            // SAFETY: a live reference the box owns; its fields outlive the reads.
            unsafe {
                assert_eq!(
                    CStr::from_ptr(ffi::git_reference_name(raw)),
                    c"refs/heads/testrepo-worktree"
                );
                assert_eq!(ffi::git_reference_type(raw), ffi::GIT_REFERENCE_DIRECT);
                assert!(ffi::git_reference_target_peel(raw).is_null());
            }
        }

        fn entry(id: &Oid) -> FetchheadEntry<'_> {
            FetchheadEntry {
                ref_name: Some(c"refs/heads/main"),
                remote_url: None,
                id: id.as_ref(),
                is_merge: false,
            }
        }

        /// The trampoline reports what the closure answered, records whether it
        /// stopped, and drops a call without an id rather than reach the closure.
        #[test]
        fn trampoline_tracks_the_closure_answers() {
            let id = Oid::zero(OidType::Sha1);
            let mut calls = Vec::new();
            let mut answer = |entry: FetchheadEntry<'_>| {
                calls.push((entry.ref_name.map(CStr::to_owned), entry.is_merge));
                if calls.len() == 2 {
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            };
            let mut callback = FetchheadForeachCallback::new(&mut answer);
            assert!(!callback.stopped());
            assert_eq!(callback.call(entry(&id)), ControlFlow::Continue(()));
            assert!(!callback.stopped());
            assert_eq!(callback.call(entry(&id)), ControlFlow::Break(()));
            assert!(callback.stopped());
            assert_eq!(callback.call(entry(&id)), ControlFlow::Continue(()));
            assert!(!callback.stopped());
            let (f, payload) = callback.raw();
            // SAFETY: the callback's own trampoline and payload; a NULL id, which
            // libgit2 never passes.
            let rc = unsafe { f.unwrap()(ptr::null(), ptr::null(), ptr::null(), 1, payload) };
            assert_eq!(rc, ErrorCode::User.raw());
            assert!(callback.stopped());
            assert_eq!(
                format!("{callback:?}"),
                "FetchheadForeachCallback { stopped: true, panicked: false, .. }"
            );
            drop(callback);
            assert_eq!(calls.len(), 3);
            assert_eq!(calls[0], (Some(c"refs/heads/main".to_owned()), false));
        }

        /// A panic of the closure is caught in the trampoline, kept, and resumed by
        /// `call` or `resume_panic`; nothing is kept once resumed.
        #[test]
        fn trampoline_keeps_a_panic_for_resumption() {
            let id = Oid::zero(OidType::Sha256);
            let mut panicking =
                |_: FetchheadEntry<'_>| -> ControlFlow<()> { panic!("from the closure") };
            let mut callback = FetchheadForeachCallback::new(&mut panicking);
            let (f, payload) = callback.raw();
            let (f, raw_id) = (f.unwrap(), id.as_ref().as_ptr());
            // SAFETY: the callback's own trampoline and payload, with a live id.
            let rc = unsafe { f(ptr::null(), ptr::null(), raw_id, 0, payload) };
            assert_eq!(rc, ErrorCode::User.raw());
            assert!(callback.stopped());
            assert!(format!("{callback:?}").contains("panicked: true"));
            let resumed = panic::catch_unwind(AssertUnwindSafe(|| callback.resume_panic()));
            assert_eq!(
                *resumed.unwrap_err().downcast::<&str>().unwrap(),
                "from the closure"
            );
            callback.resume_panic();
            assert!(format!("{callback:?}").contains("panicked: false"));
            let resumed = panic::catch_unwind(AssertUnwindSafe(|| callback.call(entry(&id))));
            assert!(resumed.is_err());
        }
    }

    mod init_ext_and_fetchhead {
        use super::super::*;
        use core::cell::Cell;
        use std::ffi::CString;
        use std::fs;
        use std::os::unix::ffi::OsStrExt;
        use std::path::{Path, PathBuf};

        fn scratch(tag: &str) -> PathBuf {
            let path = std::env::temp_dir().join(format!(
                "crustify-repository-init-ext-unit-{}-{tag}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            path
        }

        fn c_path(path: &Path) -> CString {
            CString::new(path.as_os_str().as_bytes()).unwrap()
        }

        /// The working directory a repository at `path` reopens with.
        fn reopened_workdir(path: &Path) -> Option<String> {
            let repo = Repository::open(&c_path(path)).unwrap();
            repo.as_ref()
                .workdir()
                .map(|w| w.to_str().unwrap().to_owned())
        }

        /// libgit2 reads the internal bits an earlier `git_repository_init_ext` left in
        /// reused options: after an init whose working directory was the parent of a
        /// `.git` path, an init with a working directory elsewhere writes neither the
        /// `.git` link nor `core.worktree`, and the repository reopens with the wrong
        /// working directory. The wrapper clears the bits first. C's defect is shown on
        /// a C struct alongside.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn init_ext_does_not_reuse_the_bits_of_an_earlier_call() {
            assert!(runtime_pinned());
            let root = scratch("reuse");
            for side in ["c", "rust"] {
                fs::create_dir_all(root.join(side)).unwrap();
            }
            let wd = [c_path(&root.join("c/wd")), c_path(&root.join("rust/wd"))];

            // C: the options carry `__HAS_DOTGIT | __NATURAL_WD` into the second call.
            let mut c = InitOptions::new().0;
            c.flags = ffi::GIT_REPOSITORY_INIT_MKPATH;
            for (path, workdir) in [("c/a/.git", None), ("c/sep.git", Some(&wd[0]))] {
                c.workdir_path = workdir.map_or(ptr::null(), |w| w.as_ptr());
                let path = c_path(&root.join(path));
                let mut out = ptr::null_mut();
                // SAFETY: a writable slot, a NUL-terminated path and live options whose
                // strings outlive the call.
                let rc = unsafe { ffi::git_repository_init_ext(&mut out, path.as_ptr(), &mut c) };
                assert_eq!(rc, 0);
                // SAFETY: the fresh repository, freed once.
                unsafe { ffi::git_repository_free(out) };
            }
            assert_eq!(c.flags & INIT_INTERNAL_BITS, 1 << 16 | 1 << 17);
            assert!(!root.join("c/wd/.git").exists());
            let c_workdir = reopened_workdir(&root.join("c/sep.git"));
            assert_eq!(c_workdir, Some(format!("{}/", root.join("c").display())));

            // Rust: the same options, reused, give the repository its own working
            // directory.
            let mut opts = InitOptions::new();
            opts.as_mut().set_flags(InitFlag::Mkpath.into());
            let first = Repository::init_ext(&c_path(&root.join("rust/a/.git")), opts.as_mut());
            drop(first.unwrap());
            assert_eq!(opts.as_ref().flags().unnamed_bits(), 1 << 16 | 1 << 17);
            opts.as_mut().set_workdir_path(Some(&wd[1]));
            let repo = Repository::init_ext(&c_path(&root.join("rust/sep.git")), opts.as_mut());
            drop(repo.unwrap());
            assert_eq!(opts.as_ref().flags(), InitFlags::from(InitFlag::Mkpath));
            let gitlink = fs::read_to_string(root.join("rust/wd/.git")).unwrap();
            assert!(gitlink.starts_with("gitdir: "), "{gitlink}");
            let config = fs::read_to_string(root.join("rust/sep.git/config")).unwrap();
            assert!(config.contains("worktree = "), "{config}");
            let workdir = reopened_workdir(&root.join("rust/sep.git"));
            assert_eq!(
                workdir,
                Some(format!("{}/", root.join("rust/wd").display()))
            );
            fs::remove_dir_all(&root).unwrap();
        }

        /// Internal bits a caller stores through `set_flags` are cleared too, and a
        /// reinitialisation's bit is set afresh by the call that reinitialises.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn init_ext_clears_internal_bits_the_caller_set() {
            assert!(runtime_pinned());
            let root = scratch("caller-bits");
            let wd = c_path(&root.join("wd"));
            let mut opts = InitOptions::new();
            let mut m = opts.as_mut();
            m.set_flags(InitFlags::from_bits_retain(
                InitFlag::Mkpath.raw() | INIT_INTERNAL_BITS | 1 << 20,
            ));
            m.set_workdir_path(Some(&wd));
            let path = c_path(&root.join("repo.git"));
            drop(Repository::init_ext(&path, opts.as_mut()).unwrap());
            // An unnamed bit libgit2 does not read is kept.
            assert_eq!(
                opts.as_ref().flags().bits(),
                InitFlag::Mkpath.raw() | 1 << 20
            );
            assert!(root.join("wd/.git").is_file());
            drop(Repository::init_ext(&path, opts.as_mut()).unwrap());
            assert_eq!(opts.as_ref().flags().unnamed_bits(), 1 << 18 | 1 << 20);
            fs::remove_dir_all(&root).unwrap();
        }

        /// `git_repository_init` returns 1 with a live repository when a reftable
        /// repository's `reftable` directory cannot be made; `init_ext` reads that as a
        /// failure too, and frees the repository (LSan in the sanitized runs).
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn init_ext_fails_and_frees_on_a_positive_return() {
            assert!(runtime_pinned());
            let root = scratch("reftable-file");
            let path = root.join("repo.git");
            fs::create_dir_all(path.join("objects")).unwrap();
            fs::create_dir_all(path.join("refs")).unwrap();
            fs::write(path.join("HEAD"), "ref: refs/heads/main\n").unwrap();
            fs::write(
                path.join("config"),
                "[core]\n\trepositoryformatversion = 1\n\tbare = true\n\
                 [extensions]\n\trefstorage = reftable\n",
            )
            .unwrap();
            fs::write(path.join("reftable"), "not a directory\n").unwrap();
            let mut opts = InitOptions::new();
            let mut m = opts.as_mut();
            m.set_flags(InitFlag::Bare.into());
            m.set_refdb_type(Some(RefdbType::Reftable));
            let error = Repository::init_ext(&c_path(&path), opts.as_mut()).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::Generic));
            assert!(
                error
                    .message()
                    .to_str()
                    .unwrap()
                    .starts_with("failed to make directory"),
                "{error:?}"
            );
            fs::remove_dir_all(&root).unwrap();
        }

        /// A repository without paths has no `FETCH_HEAD`: libgit2 would read the
        /// process's working directory's. Refused before the call, the callback never
        /// called.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn for_each_fetchhead_refuses_a_repository_without_paths() {
            let repo = Repository::new().unwrap();
            let calls = Cell::new(0);
            let mut count = |_: FetchheadEntry<'_>| {
                calls.set(calls.get() + 1);
                ControlFlow::Continue(())
            };
            let mut callback = FetchheadForeachCallback::new(&mut count);
            let error = repo.as_ref().for_each_fetchhead(&mut callback).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::NotFound));
            assert_eq!(error.class(), Some(ErrorClass::Invalid));
            assert_eq!(error.message(), c"path cannot exist in repository");
            assert!(!callback.stopped());
            drop(callback);
            assert_eq!(calls.get(), 0);
        }

        /// A stop is told from libgit2's errors by the callback's own record, which each
        /// walk resets: a failure after a stopped walk is an error, not a stop.
        #[test]
        #[cfg_attr(miri, ignore = "calls libgit2")]
        fn for_each_fetchhead_tells_a_stop_from_a_later_failure() {
            assert!(runtime_pinned());
            let root = scratch("stop-then-fail");
            let path = root.join("repo.git");
            let mut opts = InitOptions::new();
            opts.as_mut().set_flags(InitFlag::Bare | InitFlag::Mkdir);
            let repo = Repository::init_ext(&c_path(&path), opts.as_mut()).unwrap();
            let line = "a65fedf39aefe402d3bb6e24df4d4f5fe4547750\t\tbranch 'main' of x\n";
            fs::write(path.join("FETCH_HEAD"), line.repeat(2)).unwrap();
            let mut stop = |_: FetchheadEntry<'_>| ControlFlow::Break(());
            let mut callback = FetchheadForeachCallback::new(&mut stop);
            let walked = repo.as_ref().for_each_fetchhead(&mut callback);
            assert_eq!(walked, Ok(ControlFlow::Break(())));
            assert!(callback.stopped());
            fs::remove_file(path.join("FETCH_HEAD")).unwrap();
            let error = repo.as_ref().for_each_fetchhead(&mut callback).unwrap_err();
            assert_eq!(error.code(), Some(ErrorCode::NotFound));
            assert!(!callback.stopped());
            // A stop at a malformed line's predecessor never reaches the bad line.
            fs::write(path.join("FETCH_HEAD"), format!("{line}garbage\n")).unwrap();
            let walked = repo.as_ref().for_each_fetchhead(&mut callback);
            assert_eq!(walked, Ok(ControlFlow::Break(())));
            drop(callback);
            let mut go = |_: FetchheadEntry<'_>| ControlFlow::Continue(());
            let mut callback = FetchheadForeachCallback::new(&mut go);
            let error = repo.as_ref().for_each_fetchhead(&mut callback).unwrap_err();
            assert_eq!(error.class(), Some(ErrorClass::FetchHead));
            assert_eq!(error.message(), c"invalid object ID in FETCH_HEAD line 2");
            drop(repo);
            fs::remove_dir_all(&root).unwrap();
        }
    }
}
