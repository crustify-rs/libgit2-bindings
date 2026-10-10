//! Safe wrappers over `include/git2/mailmap.h`.
//!
//! A [`Mailmap`] is a `git_mailmap`: the parsed state of `.mailmap` files, which map
//! the names and emails commits record to the real ones. The struct is opaque (its body
//! is in `src/libgit2/mailmap.h`): one entry vector, sorted by the email and name each
//! entry replaces, whose entries and their strings the mailmap owns. libgit2 allocates
//! it in `git_mailmap_new` alone, which `git_mailmap_from_buffer` and
//! `git_mailmap_from_repository` call, and frees it, entries included, in
//! `git_mailmap_free`. It keeps no count and has no copy routine, so a mailmap has one
//! owner, a [`MailmapBox`], built by [`Mailmap::new`], [`Mailmap::from_buffer`] or
//! [`Mailmap::from_repository`], and freed on drop.
//!
//! A mailmap keeps nothing of what it was built from: `git_mailmap_from_buffer` copies
//! each entry out of the buffer, and `git_mailmap_from_repository` reads the
//! repository's config, working directory and blobs during the call only. So a
//! [`MailmapBox`] borrows neither, and outlives both.
//!
//! The routines over a mailmap go on its handles, which their own runs add: lookups
//! (`git_mailmap_resolve`, `git_mailmap_resolve_signature`, and the
//! `git_commit_*_with_mailmap` pair), which take a `const git_mailmap *`, on the shared
//! [`MailmapRef`]; `git_mailmap_add_entry`, which inserts into the entry vector, on the
//! exclusive [`MailmapMut`].
//!
//! # Threads
//!
//! A mailmap is `Send` and `Sync`. Its entries live in libgit2's allocator, which any
//! thread may free into, and it holds no thread state and no pointer to a repository.
//! Every routine taking a `const git_mailmap *` only reads it: entries are only
//! inserted with `git_vector_insert_sorted`, which keeps the vector's sorted flag that
//! `git_vector_init` sets, so the sort `git_vector_bsearch` runs before a lookup
//! returns at once without writing. The one writer, `git_mailmap_add_entry`, needs the
//! exclusive handle.

use core::fmt;
use core::ptr::{self, NonNull};
use std::borrow::Cow;

use ffibox::{CBox, CNew, define_ctype, impl_cdrop};

use crate::ffi;
use crate::libgit2::repository::repository::RepositoryRef;
use crate::libgit2::util::errors::{ErrorBuf, ErrorClass, last};
use crate::libgit2::util::sys::errors::{clear, runtime_pinned, set};

define_ctype!(
    /// A mailmap (`git_mailmap`): an opaque C object holding the entries parsed from
    /// `.mailmap` files, each mapping a name and email (or an email alone) to a real
    /// name and email.
    ///
    /// libgit2 allocates every mailmap and releases it with `git_mailmap_free`; it
    /// keeps no count, so a mailmap has one owner, a [`MailmapBox`], built by
    /// [`new`](Self::new), [`from_buffer`](Self::from_buffer) or
    /// [`from_repository`](Self::from_repository), and read and written through the
    /// box's [`as_ref`](CBox::as_ref) / [`as_mut`](CBox::as_mut) handles. There is no
    /// copy: libgit2 has none, so the box is not `Clone`.
    ///
    /// ```compile_fail,E0599
    /// use libgit2::libgit2::object::mailmap::Mailmap;
    ///
    /// let mailmap = Mailmap::new().unwrap();
    /// let _copy = mailmap.clone();
    /// ```
    ///
    /// A handle cannot outlive the box:
    ///
    /// ```compile_fail,E0505
    /// use libgit2::libgit2::object::mailmap::Mailmap;
    ///
    /// let mailmap = Mailmap::new().unwrap();
    /// let handle = mailmap.as_ref();
    /// drop(mailmap);
    /// let _ = handle;
    /// ```
    ///
    /// nor be used while an exclusive one lives:
    ///
    /// ```compile_fail,E0502
    /// use libgit2::libgit2::object::mailmap::Mailmap;
    ///
    /// let mut mailmap = Mailmap::new().unwrap();
    /// let writer = mailmap.as_mut();
    /// let reader = mailmap.as_ref();
    /// drop(writer);
    /// let _ = reader;
    /// ```
    ///
    /// The box and its handles are `Send` and `Sync`: shared access only reads (see the
    /// [module docs](self#threads)).
    ///
    /// Wraps: git_mailmap
    Mailmap, MailmapRef, MailmapMut, ffi::git_mailmap
);

// SAFETY: a mailmap owns its entries and their strings, all in libgit2's allocator,
// which any thread may allocate from and free into; it holds no thread-local state and
// no pointer to a repository or to anything else it was built from.
unsafe impl Send for Mailmap {}
// SAFETY: every routine reaching a mailmap through a shared pointer only reads it: the
// entry vector's sorted flag, set by `git_vector_init`, is kept by
// `git_vector_insert_sorted`, the only way entries are added, so the sort before each
// binary search writes nothing. The one writer, `git_mailmap_add_entry`, takes the
// exclusive handle, which excludes every other.
unsafe impl Sync for Mailmap {}
// SAFETY: `MailmapRef` is a shared borrow of a `Sync` mailmap, as `&Mailmap`.
unsafe impl Send for MailmapRef<'_> {}
// SAFETY: as above.
unsafe impl Sync for MailmapRef<'_> {}
// SAFETY: `MailmapMut` is an exclusive borrow of a `Send` mailmap, as `&mut Mailmap`.
unsafe impl Send for MailmapMut<'_> {}
// SAFETY: `&MailmapMut` only reaches the shared handle, through `as_ref`.
unsafe impl Sync for MailmapMut<'_> {}

impl fmt::Debug for MailmapRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("MailmapRef").field(&self.as_ptr()).finish()
    }
}

impl fmt::Debug for MailmapMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_ref().fmt(f)
    }
}

/// Releases a mailmap through `git_mailmap_free`: every entry and its strings, the
/// entry vector, then the struct. Also builds one through `git_mailmap_new`, so
/// [`CBox::new`] makes the empty mailmap [`Mailmap::new`] makes, reporting a failure
/// as `None`.
///
/// No mailmap reaches C before libgit2 is initialised: each constructor pins the
/// runtime first, so the free is ordered after the initialisation that settled the
/// allocator it frees into.
///
/// Wraps: git_mailmap_free
/// Wraps: git_mailmap_new
#[derive(Clone, Copy, Debug, Default)]
pub struct MailmapFree;

impl_cdrop!(MailmapFree, Mailmap, ffi::git_mailmap_free);

// SAFETY: `git_mailmap_new` writes a fresh `git__calloc`'d mailmap with an empty,
// sorted entry vector, which the caller owns alone and `git_mailmap_free` releases; it
// writes `out` only on success, so a failure (out of memory) leaves it NULL, which is
// `None`. The runtime is pinned before the call.
unsafe impl CNew<Mailmap> for MailmapFree {
    fn c_new(&self) -> Option<NonNull<Mailmap>> {
        if !runtime_pinned() {
            return None;
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; the runtime is pinned, so the allocation
        // goes through an initialised allocator and a failure is reported on live
        // thread-local state.
        let rc = unsafe { ffi::git_mailmap_new(&mut out) };
        if rc < 0 {
            return None;
        }
        NonNull::new(out.cast())
    }
}

/// The owner of a [`Mailmap`], freed by `git_mailmap_free` on drop.
pub type MailmapBox = CBox<Mailmap, MailmapFree>;

impl Mailmap {
    /// An empty mailmap: every lookup resolves a name and email to themselves.
    ///
    /// Pins the runtime first. Fails, with libgit2's error, when the mailmap cannot be
    /// allocated or libgit2 cannot be initialised.
    ///
    /// Wraps: git_mailmap_new
    pub fn new() -> Result<MailmapBox, ErrorBuf> {
        MailmapBox::new().ok_or_else(last)
    }

    /// The mailmap of `buf`, the contents of a `.mailmap` file: one entry per line,
    /// `Real Name <real@email>` followed by the `Name <email>` (or `<email>` alone) it
    /// replaces, either real part optional. Lines that are not an entry, comments
    /// (`#`) included, are skipped; a later entry for the same name and email replaces
    /// an earlier one. The entries are copied: the mailmap does not borrow `buf`.
    ///
    /// A line whose replaced email is empty (`Name <>`, `Real <real@email> Name <>`)
    /// is skipped too, as any other line that is not an entry. libgit2 diverges here:
    /// it begins such an entry, then fails the whole buffer with
    /// [`ErrorClass::Invalid`], "invalid argument: 'replace_email && *replace_email'",
    /// and leaks the entry; so those lines are left out of what it is given.
    ///
    /// Pins the runtime first. Fails when `buf` contains a NUL byte, which libgit2
    /// refuses without an error of its own, reported as [`ErrorClass::Invalid`],
    /// "mailmap buffer contains a NUL byte"; and with libgit2's error when the mailmap
    /// cannot be allocated or libgit2 cannot be initialised.
    ///
    /// Wraps: git_mailmap_from_buffer
    pub fn from_buffer(buf: &[u8]) -> Result<MailmapBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let buf = without_empty_replacements(buf);
        // An empty slice's pointer is dangling, and libgit2 hands it to `memchr`, whose
        // pointer must be valid even for a length of 0: lend a live byte instead.
        let data = if buf.is_empty() {
            c"".as_ptr()
        } else {
            buf.as_ptr().cast()
        };
        let mut out = ptr::null_mut();
        clear();
        // SAFETY: `out` is a writable slot; `data` addresses `buf.len()` readable bytes
        // (or one, for an empty `buf`), which libgit2 only reads during the call. The
        // runtime is pinned, so allocations and errors reach live state.
        let rc = unsafe { ffi::git_mailmap_from_buffer(&mut out, data, buf.len()) };
        if rc < 0 {
            // The error state was cleared before the call, so `GIT_ERROR_NONE` is
            // libgit2's silence: the NUL check, the one failure that sets no error.
            if last().class() == Some(ErrorClass::None) {
                set(ErrorClass::Invalid, c"mailmap buffer contains a NUL byte");
            }
            return Err(last());
        }
        // SAFETY: on success `out` holds a fresh mailmap the caller owns alone, built by
        // `git_mailmap_new` and released by `git_mailmap_free`.
        unsafe { MailmapBox::from_c(out) }.ok_or_else(last)
    }

    /// The mailmap of `repo`, as git builds it, later sources overriding earlier ones
    /// entry by entry:
    ///
    /// 1. the `.mailmap` file at the root of the working directory, for a repository
    ///    that is not bare;
    /// 2. the blob that the `mailmap.blob` setting names, by default `HEAD:.mailmap` in
    ///    a bare repository and none otherwise;
    /// 3. the file that the `mailmap.file` setting names.
    ///
    /// A source that is missing or cannot be read or parsed is skipped, as libgit2
    /// skips it, without an error. Unlike [`from_buffer`](Self::from_buffer), this
    /// cannot leave out the lines whose replaced email is empty (`Name <>`), since
    /// libgit2 reads the sources itself: on the first such line it drops the rest of
    /// that source and leaks the entry it began (a C defect).
    ///
    /// The mailmap keeps nothing of `repo`, so it outlives the repository; the call
    /// fills the repository's config cache, as every routine reading its config does,
    /// through the shared handle.
    ///
    /// Pins the runtime first. Fails, with libgit2's error, only when the mailmap
    /// cannot be allocated or libgit2 cannot be initialised.
    ///
    /// Wraps: git_mailmap_from_repository
    pub fn from_repository(repo: RepositoryRef<'_>) -> Result<MailmapBox, ErrorBuf> {
        if !runtime_pinned() {
            return Err(last());
        }
        let mut out = ptr::null_mut();
        // SAFETY: `out` is a writable slot; `repo` is a live repository, borrowed for
        // the call, which reads it and fills its caches through the pointer: the
        // repository is not `Sync`, so no other thread uses it meanwhile. The runtime is
        // pinned, so allocations and errors reach live state.
        let rc = unsafe { ffi::git_mailmap_from_repository(&mut out, repo.as_ptr().cast_mut()) };
        if rc < 0 {
            return Err(last());
        }
        // SAFETY: on success `out` holds a fresh mailmap the caller owns alone, built by
        // `git_mailmap_new` and released by `git_mailmap_free`; it keeps no pointer to
        // `repo`.
        unsafe { MailmapBox::from_c(out) }.ok_or_else(last)
    }
}

/// `buf` without the lines libgit2's mailmap parser would read as an entry replacing an
/// empty email, which `mailmap_add_entry_unterminated` refuses with a failed
/// `GIT_ASSERT_ARG` after allocating the entry, failing the buffer and leaking it.
///
/// A buffer with a NUL byte is kept whole, since libgit2 refuses it before parsing; so
/// is one without `<>`, the only spelling of an empty email. A copy is made only when a
/// line is left out.
fn without_empty_replacements(buf: &[u8]) -> Cow<'_, [u8]> {
    if buf.contains(&0) || !buf.windows(2).any(|w| w == b"<>") {
        return Cow::Borrowed(buf);
    }
    let lines = || buf.split_inclusive(|&c| c == b'\n');
    if !lines().any(replaces_empty_email) {
        return Cow::Borrowed(buf);
    }
    Cow::Owned(
        lines()
            .filter(|line| !replaces_empty_email(line))
            .flatten()
            .copied()
            .collect(),
    )
}

/// Whether `parse_mailmap_entry` reads `line` (one line, with its `\n` if any) as an
/// entry whose replaced email, the last `<...>` of the line, is empty.
///
/// Mirrors the parser step by step: leading blanks are skipped (`git_parse_advance_ws`:
/// `git__isspace` but `\n`); the line ends where only whitespace or a `#` comment is
/// left (`is_eol`); and each `<` or `>` is searched up to the end of the line or the
/// first `#`, which fails the line (`advance_until`). A line the parser rejects, or
/// whose replaced email is not empty, is not such an entry.
fn replaces_empty_email(line: &[u8]) -> bool {
    let is_space = |c: u8| matches!(c, b' ' | b'\t' | b'\n' | b'\x0c' | b'\r' | b'\x0b');
    let skip_blanks = |rest: &'_ [u8]| -> usize {
        rest.iter()
            .take_while(|&&c| c != b'\n' && is_space(c))
            .count()
    };
    let at_eol = |rest: &[u8]| {
        rest.iter()
            .find(|&&c| !is_space(c))
            .is_none_or(|&c| c == b'#')
    };
    let until = |rest: &[u8], needle: u8| {
        rest.iter()
            .position(|&c| c == b'#' || c == needle)
            .filter(|&i| rest[i] == needle)
    };

    let mut rest = &line[skip_blanks(line)..];
    if at_eol(rest) {
        return false;
    }
    let Some(open) = until(rest, b'<') else {
        return false;
    };
    rest = &rest[open + 1..];
    let Some(close) = until(rest, b'>') else {
        return false;
    };
    let mut replaced_len = close;
    rest = &rest[close + 1..];
    if !at_eol(rest) {
        rest = &rest[skip_blanks(rest)..];
        let Some(open) = until(rest, b'<') else {
            return false;
        };
        rest = &rest[open + 1..];
        let Some(close) = until(rest, b'>') else {
            return false;
        };
        replaced_len = close;
        rest = &rest[close + 1..];
    }
    at_eol(rest) && replaced_len == 0
}

// A `Mailmap` is only ever behind a pointer, so the box is pointer-sized, and an
// optional one is a nullable `git_mailmap *`.
const _: () = assert!(
    size_of::<MailmapBox>() == size_of::<*mut ffi::git_mailmap>()
        && size_of::<Option<MailmapBox>>() == size_of::<*mut ffi::git_mailmap>()
);

#[cfg(test)]
mod unit_tests {
    use super::*;
    use crate::libgit2::repository::repository::Repository;
    use crate::libgit2::util::sys::alloc::{Alloc, Allocator};
    use core::ffi::{CStr, c_char, c_int, c_void};
    use core::mem::size_of;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
    use std::thread;

    const CHILD: &str = "CRUSTIFY_UNIT_CHILD";

    /// Runs `body` alone in a child process, where nothing has initialised libgit2.
    fn isolated(test: &str, body: fn()) {
        isolated_with(test, &[], body);
    }

    /// As [`isolated`], with `env` set in the child.
    fn isolated_with(test: &str, env: &[(&str, &str)], body: fn()) {
        let name = test.split_once("::").map_or(test, |(_, rest)| rest);
        if std::env::var(CHILD).as_deref() == Ok(name) {
            body();
            return;
        }
        let out = Command::new(std::env::current_exe().unwrap())
            .args([name, "--exact", "--test-threads=1", "--nocapture"])
            .env(CHILD, name)
            .envs(env.iter().copied())
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
    /// While `REFUSING` is set, the allocations still granted before refusing.
    static GRANTS: AtomicUsize = AtomicUsize::new(0);

    /// Whether an allocation is refused: while `REFUSING` is set, once `GRANTS` is
    /// spent.
    fn refused() -> bool {
        REFUSING.load(SeqCst)
            && GRANTS
                .try_update(SeqCst, SeqCst, |g| g.checked_sub(1))
                .is_err()
    }

    unsafe extern "C" {
        fn malloc(n: usize) -> *mut c_void;
        fn realloc(p: *mut c_void, n: usize) -> *mut c_void;
        fn free(p: *mut c_void);
    }

    /// libc's allocator, refusing what [`refused`] says.
    struct Refusing;

    // SAFETY: forwards to libc's `malloc` family, or fails, which `Alloc` allows.
    unsafe impl Alloc for Refusing {
        fn malloc(len: usize, _: Option<&CStr>, _: i32) -> Option<NonNull<u8>> {
            if refused() {
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
            if refused() {
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

    fn install_refusing() {
        let allocator = Allocator::new::<Refusing>();
        // SAFETY: `Allocator` is `#[repr(transparent)]` over `git_allocator`, with every
        // member set; libgit2 copies it before anything is allocated.
        let rc = unsafe {
            ffi::git_libgit2_opts(
                ffi::GIT_OPT_SET_ALLOCATOR as c_int,
                allocator.as_ref().as_ptr(),
            )
        };
        assert_eq!(rc, 0);
    }

    /// `git_mailmap_resolve` on `mailmap`: the real name and email of `name <email>`,
    /// copied out.
    fn resolve(mailmap: MailmapRef<'_>, name: &CStr, email: &CStr) -> (String, String) {
        let (mut real_name, mut real_email): (*const c_char, *const c_char) =
            (ptr::null(), ptr::null());
        // SAFETY: two writable slots; `mailmap` is live for the call, which only reads
        // it; `name` and `email` are NUL-terminated.
        let rc = unsafe {
            ffi::git_mailmap_resolve(
                &mut real_name,
                &mut real_email,
                mailmap.as_ptr(),
                name.as_ptr(),
                email.as_ptr(),
            )
        };
        assert_eq!(rc, 0);
        // SAFETY: on success both point at `name`, `email` or an entry's strings, all
        // NUL-terminated and live until `mailmap` or the inputs are released, after
        // this copy.
        unsafe {
            (
                CStr::from_ptr(real_name).to_string_lossy().into_owned(),
                CStr::from_ptr(real_email).to_string_lossy().into_owned(),
            )
        }
    }

    fn pair(name: &str, email: &str) -> (String, String) {
        (name.to_owned(), email.to_owned())
    }

    const MAILMAP: &[u8] = b"\
# a comment
Real Name <real@example.com> <old@example.com>
<fixed@example.com> Some Name <some@example.com>
Other Real <other@example.com> Other Old <other.old@example.com>
not an entry
";

    #[test]
    fn layout_matches_c() {
        assert_eq!(size_of::<MailmapBox>(), size_of::<*mut ffi::git_mailmap>());
        assert_eq!(
            size_of::<Option<MailmapBox>>(),
            size_of::<*mut ffi::git_mailmap>()
        );
        assert_eq!(
            size_of::<MailmapRef<'_>>(),
            size_of::<*const ffi::git_mailmap>()
        );
        assert_eq!(
            size_of::<MailmapMut<'_>>(),
            size_of::<*const ffi::git_mailmap>()
        );
        assert_eq!(size_of::<MailmapFree>(), 0);
    }

    /// The handles wrap a pointer without reading through it: `from_ptr` refuses NULL
    /// and keeps any other pointer, `as_ptr` gives it back, and `Debug` prints it.
    #[test]
    fn handles_wrap_pointers_without_reading() {
        let dangling: *mut ffi::git_mailmap = NonNull::dangling().as_ptr();
        // SAFETY: the handle is never dereferenced: `as_ptr` and `Debug` only read the
        // pointer itself, which is what this test checks.
        let handle = unsafe { MailmapRef::from_ptr(dangling) }.unwrap();
        assert_eq!(handle.as_ptr(), dangling.cast_const());
        assert_eq!(format!("{handle:?}"), format!("MailmapRef({dangling:p})"));
        // SAFETY: NULL is refused before any handle exists.
        assert!(unsafe { MailmapRef::from_ptr(ptr::null_mut()) }.is_none());
    }

    /// The exclusive handle reborrows shared and exclusive, and prints as the shared
    /// one.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn exclusive_handle_reborrows() {
        let mut sole = Mailmap::new().unwrap();
        let raw = sole.as_c_ptr();
        let mut m = sole.as_mut();
        assert_eq!(m.as_mut().as_mut_ptr(), raw);
        assert_eq!(m.as_ref().as_ptr(), raw.cast_const());
        assert_eq!(format!("{m:?}"), format!("MailmapRef({raw:p})"));
        // SAFETY: `m` is the only handle to a live mailmap, written for the call; the
        // strings are NUL-terminated and copied.
        let rc = unsafe {
            ffi::git_mailmap_add_entry(
                m.as_mut_ptr(),
                c"Real".as_ptr(),
                ptr::null(),
                ptr::null(),
                c"x@example.com".as_ptr(),
            )
        };
        assert_eq!(rc, 0);
        assert_eq!(
            resolve(sole.as_ref(), c"Any", c"x@example.com"),
            pair("Real", "x@example.com")
        );
    }

    /// Every construction path builds a mailmap that resolves as its source says, and is
    /// freed once (LSan flags a leak, ASan a double free).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn constructors_build_mailmaps() {
        let empty = Mailmap::new().unwrap();
        assert_eq!(
            resolve(empty.as_ref(), c"A", c"a@example.com"),
            pair("A", "a@example.com")
        );
        let through_policy = MailmapBox::new().unwrap();
        assert_ne!(through_policy.as_c_ptr(), empty.as_c_ptr());

        let parsed = Mailmap::from_buffer(MAILMAP).unwrap();
        let m = parsed.as_ref();
        assert_eq!(
            resolve(m, c"Anyone", c"old@example.com"),
            pair("Real Name", "real@example.com")
        );
        assert_eq!(
            resolve(m, c"Some Name", c"some@example.com"),
            pair("Some Name", "fixed@example.com")
        );
        assert_eq!(
            resolve(m, c"Other Old", c"other.old@example.com"),
            pair("Other Real", "other@example.com")
        );
        assert_eq!(
            resolve(m, c"Someone Else", c"other.old@example.com"),
            pair("Someone Else", "other.old@example.com")
        );

        // An empty buffer is lent as one live byte, and parses to no entries.
        let none = Mailmap::from_buffer(b"").unwrap();
        assert_eq!(
            resolve(none.as_ref(), c"A", c"old@example.com"),
            pair("A", "old@example.com")
        );
    }

    /// The buffer is copied: the mailmap outlives it.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn mailmap_outlives_its_buffer() {
        let buf = MAILMAP.to_vec();
        let parsed = Mailmap::from_buffer(&buf).unwrap();
        drop(buf);
        assert_eq!(
            resolve(parsed.as_ref(), c"X", c"old@example.com"),
            pair("Real Name", "real@example.com")
        );
    }

    /// libgit2 refuses a NUL byte without an error of its own; the wrapper reports one,
    /// rather than whatever error an earlier call left.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn nul_bytes_are_refused_with_an_error() {
        set(ErrorClass::Odb, c"an earlier error");
        let error = Mailmap::from_buffer(b"A <a@example.com>\0<b@example.com>\n").unwrap_err();
        assert_eq!(error.class(), Some(ErrorClass::Invalid));
        assert_eq!(error.message(), c"mailmap buffer contains a NUL byte");
        assert_eq!(last(), error);
    }

    /// A box adopted from C is released once, by `git_mailmap_free`; one given back
    /// with `into_c` is C's to free.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn adoption_round_trips() {
        assert!(runtime_pinned());
        let mut raw = ptr::null_mut();
        // SAFETY: a writable slot; the buffer is `MAILMAP.len()` readable bytes.
        let rc = unsafe {
            ffi::git_mailmap_from_buffer(&mut raw, MAILMAP.as_ptr().cast(), MAILMAP.len())
        };
        assert_eq!(rc, 0);
        // SAFETY: a fresh mailmap the caller owns alone, released by `git_mailmap_free`.
        let adopted = unsafe { MailmapBox::from_c(raw) }.unwrap();
        assert_eq!(
            resolve(adopted.as_ref(), c"X", c"old@example.com"),
            pair("Real Name", "real@example.com")
        );
        let given_back = adopted.into_c();
        assert_eq!(given_back, raw);
        // SAFETY: `into_c` surrendered the only reference, freed here once.
        unsafe { ffi::git_mailmap_free(given_back) };
        // SAFETY: NULL is refused before any box exists.
        assert!(unsafe { MailmapBox::from_c(ptr::null_mut()) }.is_none());
    }

    /// A repository without paths has no working directory and no `HEAD`: libgit2 skips
    /// every source, and the mailmap is empty, which ASan checks reads nothing it
    /// should not.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn pathless_repositories_give_empty_mailmaps() {
        let repo = Repository::new().unwrap();
        let mailmap = Mailmap::from_repository(repo.as_ref()).unwrap();
        drop(repo);
        assert_eq!(
            resolve(mailmap.as_ref(), c"A", c"a@example.com"),
            pair("A", "a@example.com")
        );
    }

    /// Lookups through shared handles on several threads at once race nothing (TSan):
    /// they only read, as `Sync` claims, and the box is dropped on another thread
    /// (`Send`).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn shared_lookups_race_nothing() {
        let parsed = Mailmap::from_buffer(MAILMAP).unwrap();
        thread::scope(|s| {
            for _ in 0..4 {
                s.spawn(|| {
                    for _ in 0..64 {
                        assert_eq!(
                            resolve(parsed.as_ref(), c"X", c"old@example.com"),
                            pair("Real Name", "real@example.com")
                        );
                        assert_eq!(
                            resolve(parsed.as_ref(), c"Y", c"none@example.com"),
                            pair("Y", "none@example.com")
                        );
                    }
                });
            }
        });
        thread::spawn(move || drop(parsed)).join().unwrap();
    }

    /// An allocation failure is `None` from the policy and libgit2's out-of-memory
    /// error from each constructor; nothing leaks (LSan).
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn out_of_memory_is_an_error() {
        isolated(
            concat!(module_path!(), "::out_of_memory_is_an_error"),
            || {
                install_refusing();
                assert!(runtime_pinned());
                let repo = Repository::new().unwrap();
                // A first error allocates the thread's error state, which an
                // allocation failure would otherwise report instead (`GIT_ERROR_THREAD`).
                assert!(Mailmap::from_buffer(b"\0").is_err());
                REFUSING.store(true, SeqCst);
                let boxed = MailmapBox::new().is_none();
                let new = Mailmap::new().unwrap_err();
                let parsed = Mailmap::from_buffer(MAILMAP).unwrap_err();
                let loaded = Mailmap::from_repository(repo.as_ref()).unwrap_err();
                REFUSING.store(false, SeqCst);
                assert!(boxed);
                for error in [new, parsed, loaded] {
                    assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                }
            },
        );
    }

    /// An allocation failure at each point of building a mailmap, in turn, is
    /// `libgit2`'s out-of-memory error, or, once enough is granted, the whole mailmap;
    /// ASan checks every partial build is unwound without a bad access. libgit2 leaks
    /// the entry it is filling when one of its strings cannot be copied
    /// (`GIT_ERROR_CHECK_ALLOC` returns before `mailmap_entry_free`), so this runs with
    /// LeakSanitizer off.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn each_allocation_failure_is_an_error() {
        isolated_with(
            concat!(module_path!(), "::each_allocation_failure_is_an_error"),
            &[("ASAN_OPTIONS", "detect_leaks=0:abort_on_error=1")],
            || {
                install_refusing();
                assert!(runtime_pinned());
                // A first error allocates the thread's error state, which an
                // allocation failure would otherwise report instead (`GIT_ERROR_THREAD`).
                assert!(Mailmap::from_buffer(b"\0").is_err());
                let attempt = |grants: usize, build: fn() -> Result<MailmapBox, ErrorBuf>| {
                    GRANTS.store(grants, SeqCst);
                    REFUSING.store(true, SeqCst);
                    let built = build();
                    REFUSING.store(false, SeqCst);
                    built
                };
                // Nine entries: one more than the entry vector's first allocation, so
                // the vector grows too.
                const NINE: &[u8] = b"R1 <r1@x> <o1@x>\nR2 <r2@x> <o2@x>\nR3 <r3@x> <o3@x>\n\
                    R4 <r4@x> <o4@x>\nR5 <r5@x> <o5@x>\nR6 <r6@x> <o6@x>\n\
                    R7 <r7@x> <o7@x>\nR8 <r8@x> <o8@x>\nR9 <r9@x> <o9@x>\n";
                let builders: [fn() -> Result<MailmapBox, ErrorBuf>; 3] = [
                    Mailmap::new,
                    || Mailmap::from_buffer(MAILMAP),
                    || Mailmap::from_buffer(NINE),
                ];
                for build in builders {
                    let mut failures = 0;
                    let built = (0..).find_map(|grants| match attempt(grants, build) {
                        Ok(mailmap) => Some(mailmap),
                        Err(error) => {
                            assert_eq!(error.class(), Some(ErrorClass::NoMemory));
                            failures += 1;
                            None
                        }
                    });
                    assert!(failures > 0);
                    let mailmap = built.unwrap();
                    assert_eq!(
                        resolve(mailmap.as_ref(), c"A", c"a@example.com"),
                        pair("A", "a@example.com")
                    );
                }
            },
        );
    }

    /// Without libgit2's runtime nothing is built, and the error is synthesised without
    /// calling libgit2.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn without_runtime_fails_cleanly() {
        isolated(
            concat!(module_path!(), "::without_runtime_fails_cleanly"),
            || {
                REFUSING.store(true, SeqCst);
                install_refusing();
                assert!(MailmapBox::new().is_none());
                let new = Mailmap::new().unwrap_err();
                let parsed = Mailmap::from_buffer(MAILMAP).unwrap_err();
                REFUSING.store(false, SeqCst);
                assert!(!runtime_pinned(), "libgit2 could not initialise");
                for error in [new, parsed] {
                    assert_eq!(error.class(), Some(ErrorClass::Invalid));
                    assert_eq!(error.message(), c"library has not been initialized");
                }
            },
        );
    }

    /// Lines built from the tokens the parser tells apart: blanks, names, emails, an
    /// empty email, comments and stray brackets, up to four of them.
    fn corpus() -> Vec<Vec<u8>> {
        const TOKENS: [&[u8]; 11] = [
            b"", b" ", b"\t", b"Name ", b"<a@x>", b"<>", b" <>", b"# c", b"<a#b>", b"<", b">",
        ];
        // BorrowSanitizer tracks every allocation, and tens of thousands of small ones
        // take it hours: under it, only every 37th combination is built at all.
        let stride = if cfg!(bsan) { 37 } else { 1 };
        let mut lines = Vec::new();
        let mut n = 0usize;
        for a in TOKENS {
            for b in TOKENS {
                for c in TOKENS {
                    for d in TOKENS {
                        n += 1;
                        if !n.is_multiple_of(stride) {
                            continue;
                        }
                        let line = [a, b, c, d].concat();
                        lines.push([&line[..], b"\n"].concat());
                        lines.push([&line[..], b"\r\n"].concat());
                        lines.push(line);
                    }
                }
            }
        }
        lines.sort();
        lines.dedup();
        lines
    }

    /// The classifier picks exactly the lines libgit2 fails on with its
    /// `replace_email` assertion, and libgit2 accepts every other line (skipping those
    /// that are not entries). C leaks the entry it began for each refused line, so this
    /// runs in a child with LeakSanitizer off.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn classifier_agrees_with_libgit2() {
        isolated_with(
            concat!(module_path!(), "::classifier_agrees_with_libgit2"),
            &[("ASAN_OPTIONS", "detect_leaks=0:abort_on_error=1")],
            || {
                assert!(runtime_pinned());
                let mut refused = 0;
                for line in corpus() {
                    let mut out = ptr::null_mut();
                    // SAFETY: a writable slot; `line` is `line.len()` readable bytes
                    // (an empty one is lent as a live byte).
                    let rc = unsafe {
                        let data = if line.is_empty() {
                            c"".as_ptr()
                        } else {
                            line.as_ptr().cast()
                        };
                        ffi::git_mailmap_from_buffer(&mut out, data, line.len())
                    };
                    let shown = String::from_utf8_lossy(&line);
                    if replaces_empty_email(&line) {
                        refused += 1;
                        assert!(rc < 0 && out.is_null(), "{shown:?}");
                        let message = last().message().to_string_lossy().into_owned();
                        assert!(message.contains("replace_email"), "{shown:?}: {message}");
                    } else {
                        assert_eq!(rc, 0, "{shown:?}");
                        // SAFETY: a fresh mailmap, freed once.
                        unsafe { ffi::git_mailmap_free(out) };
                    }
                }
                // The corpus is sampled under BorrowSanitizer (see `corpus`).
                assert!(
                    refused > if cfg!(bsan) { 2 } else { 100 },
                    "the corpus reaches the defect"
                );
            },
        );
    }

    /// Only the refused lines are left out, and only then is the buffer copied.
    #[test]
    fn only_refused_lines_are_left_out() {
        let kept: &[u8] = b"A <> <a@x>\nB <b@x>\n";
        assert!(matches!(without_empty_replacements(kept), Cow::Borrowed(b) if b == kept));
        let with_nul: &[u8] = b"Name <>\n\0";
        assert!(matches!(
            without_empty_replacements(with_nul),
            Cow::Borrowed(_)
        ));
        let mixed = b"Name <>\nA <> <a@x>\nReal <r@x> Name <>\n  # <>\nB <b@x>";
        assert_eq!(
            &*without_empty_replacements(mixed),
            b"A <> <a@x>\n  # <>\nB <b@x>"
        );
    }

    /// A line replacing an empty email is skipped like any line that is not an entry:
    /// the rest of the buffer is read, and nothing leaks (LSan), where libgit2 fails
    /// the whole buffer and leaks the entry it began.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn empty_replaced_emails_are_skipped() {
        let parsed = Mailmap::from_buffer(
            b"Name <>\nReal Name <real@example.com> <old@example.com>\nR <r@x> N <>\n",
        )
        .unwrap();
        assert_eq!(
            resolve(parsed.as_ref(), c"X", c"old@example.com"),
            pair("Real Name", "real@example.com")
        );
        assert_eq!(resolve(parsed.as_ref(), c"N", c""), pair("N", ""));
    }
}
