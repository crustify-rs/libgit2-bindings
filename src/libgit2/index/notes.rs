//! Safe wrappers over `include/git2/notes.h`.
//!
//! Only the note-iterator alias of the internal `git_iterator` is public. Its
//! private fields and other iterator kinds are deliberately not exposed. C owns
//! the iterator, its tree references, path buffers and traversal stack. It borrows
//! the repository without counting a reference; `'r` keeps that repository alive.

use core::ffi::CStr;
use core::marker::PhantomData;
use core::ptr::{self, NonNull};
use std::ffi::CString;

use ffibox::{CBorrowedPtr, CBox, CCell, CDrop, CZeroable};

use crate::ffi;
use crate::libgit2::object::commit::{Commit, CommitRef};
use crate::libgit2::oid::oid::Oid;
use crate::libgit2::repository::repository::{RepositoryError, RepositoryRef};
use crate::libgit2::util::errors::ErrorCode;

/// An opaque note iterator. Construct with [`new`](Self::new) or
/// [`from_commit`](Self::from_commit), never as an inline value.
///
/// C retains its own root-tree reference, so the input commit can be dropped after
/// construction. The repository cannot be dropped or exclusively borrowed while
/// this iterator remains usable. There is no public clone, reset or refcount API.
/// Neither iterator nor handles are Send/Sync: traversal mutates unsynchronized
/// state and reaches the thread-bound repository.
///
/// ```compile_fail,E0505
/// use libgit2::libgit2::index::notes::NoteIterator;
/// use libgit2::libgit2::repository::repository::Repository;
/// let repo = Repository::open(c".").unwrap();
/// let it = NoteIterator::new(repo.as_ref(), None).unwrap();
/// drop(repo);
/// let _ = it.as_ref();
/// ```
///
/// ```compile_fail,E0277
/// use libgit2::libgit2::index::notes::NoteIteratorBox;
/// fn send<T: Send>() {}
/// send::<NoteIteratorBox<'static>>();
/// ```
///
/// Wraps: `git_iterator`
/// Handwritten
#[repr(transparent)]
pub struct NoteIterator<'r>(ffi::git_note_iterator, PhantomData<(*const (), &'r ())>);

/// Shared handle; the public C API has no read-only iterator operations.
/// Handwritten
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct NoteIteratorRef<'a, 'r>(CBorrowedPtr<'a, NoteIterator<'r>>);

/// Exclusive handle, required for stateful traversal.
/// Handwritten
#[repr(transparent)]
pub struct NoteIteratorMut<'a, 'r>(
    NoteIteratorRef<'a, 'r>,
    PhantomData<&'a mut NoteIterator<'r>>,
);

// SAFETY: the layout is transparent over the opaque FFI type, and both handles
// are transparent over CBorrowedPtr with no Drop. The exclusive handle is
// invariant in the pointee (including its repository lifetime). No C reference
// or inline opaque value is ever constructed.
unsafe impl<'r> CCell for NoteIterator<'r> {
    type C = ffi::git_note_iterator;
    type Mut<'a>
        = NoteIteratorMut<'a, 'r>
    where
        Self: 'a;
    type Ref<'a>
        = NoteIteratorRef<'a, 'r>
    where
        Self: 'a;
}

impl NoteIteratorRef<'_, '_> {
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_ptr(&self) -> *const ffi::git_note_iterator {
        self.0.as_non_null().as_ptr().cast()
    }
}

impl<'r> NoteIteratorMut<'_, 'r> {
    /// Reborrows shared for no longer than this handle.
    #[must_use]
    pub fn as_ref(&self) -> NoteIteratorRef<'_, 'r> {
        self.0
    }

    /// Reborrows exclusively; the original handle is frozen until this expires.
    pub fn as_mut(&mut self) -> NoteIteratorMut<'_, 'r> {
        NoteIteratorMut(self.0, PhantomData)
    }

    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only the unit tests reach it until a consumer needs it"
        )
    )]
    pub(crate) fn as_mut_ptr(&mut self) -> *mut ffi::git_note_iterator {
        self.0.0.as_non_null().as_ptr().cast()
    }
}

/// Releases the iterator's owned traversal storage and tree references through C.
/// No external state is needed and no repository reference is released.
///
/// Wraps: `git_note_iterator_free`
/// Handwritten
#[derive(Clone, Copy, Debug, Default)]
pub struct NoteIteratorFree;

// SAFETY: this policy releases exactly the sole iterator allocation returned by
// the two note constructors; C dispatches the tree-specific disposer then frees
// the common buffers and the allocation. No Rust allocation is passed to it.
unsafe impl<'r> CDrop<NoteIterator<'r>> for NoteIteratorFree {
    unsafe fn c_drop(&self, ptr: NonNull<NoteIterator<'r>>) {
        // SAFETY: the caller transfers the live allocation once; transparent
        // NoteIterator preserves its C pointer and the repository is still live.
        unsafe { ffi::git_note_iterator_free(ptr.as_ptr().cast()) }
    }
}

/// Sole owner of a note iterator, borrowing its repository for `'r`.
///
/// ```compile_fail,E0502
/// use libgit2::libgit2::index::notes::NoteIteratorBox;
/// fn alias(mut it: NoteIteratorBox<'_>) {
///     let mut exclusive = it.as_mut();
///     let shared = exclusive.as_ref();
///     let second = exclusive.as_mut();
///     let _ = (shared, second);
/// }
/// ```
pub type NoteIteratorBox<'r> = CBox<NoteIterator<'r>, NoteIteratorFree>;

/// `GIT_NOTES_DEFAULT_REF`, the notes reference when `core.notesRef` is unset.
const DEFAULT_REF: &CStr = c"refs/notes/commits";

/// `git_note_default_ref`'s answer: `core.notesRef`, valueless read as empty, else
/// [`DEFAULT_REF`]; any other lookup failure is returned.
fn default_ref(repo: RepositoryRef<'_>) -> Result<CString, RepositoryError> {
    let config = repo.config()?;
    match config.as_ref().entry(c"core.notesRef") {
        Ok(entry) => Ok(entry.as_ref().value().unwrap_or(c"").to_owned()),
        Err(error) if error.code() == Some(ErrorCode::NotFound) => Ok(DEFAULT_REF.to_owned()),
        Err(error) => Err(RepositoryError::from_parts(
            error.raw_code(),
            error.error().clone(),
        )),
    }
}

impl<'r> NoteIterator<'r> {
    /// Opens notes at `notes_ref`, or at the repository's default notes reference (`core.notesRef`,
    /// else `refs/notes/commits`). The name is read during the call, not retained.
    ///
    /// Composed in Rust: `git_note_iterator_new` ignores a failed commit lookup
    /// (reference to a missing object or a non-commit) and then frees an
    /// uninitialised tree pointer, so the name is resolved here and the commit handed
    /// to [`from_commit`](Self::from_commit), with the lookup's own error. The default
    /// name is read through [`ConfigRef::entry`], not `git_note_default_ref`, which
    /// frees an uninitialised config entry when the configuration file no longer
    /// parses; that failure is returned instead.
    ///
    /// [`ConfigRef::entry`]: crate::libgit2::config::config::ConfigRef::entry
    ///
    /// Wraps: `git_note_iterator_new`
    ///
    /// # Errors
    ///
    /// A missing or invalid reference, a target that is not a commit, a notes tree missing from the
    /// object database and allocation failures return libgit2's error.
    pub fn new(
        repo: RepositoryRef<'r>,
        notes_ref: Option<&CStr>,
    ) -> Result<NoteIteratorBox<'r>, RepositoryError> {
        let default;
        let name = if let Some(name) = notes_ref {
            name
        } else {
            default = default_ref(repo)?;
            default.as_c_str()
        };
        let mut id = Oid::zeroed();
        // SAFETY: `id` is a writable id, set only on success; `repo` is live on this
        // thread and `name` a NUL string read for the call only. Raw seam until
        // `git_reference_name_to_id` has a safe wrapper.
        let rc = unsafe {
            ffi::git_reference_name_to_id(
                id.as_mut().as_mut_ptr(),
                repo.as_ptr().cast_mut(),
                name.as_ptr(),
            )
        };
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        let commit = Commit::lookup(repo, id.as_ref())?;
        Self::from_commit(commit.as_ref())
    }

    /// Opens the notes tree of a commit, retaining the tree but not the commit.
    /// The result can outlive the commit, never its repository. A tree missing from
    /// the object database and allocation failures return libgit2's error.
    ///
    /// `git_note_commit_iterator_new` frees an uninitialised pointer when its tree
    /// lookup fails, so the tree is looked up here first and held over the call. Its
    /// lookup then hits the object cache or rereads an object just read: only an
    /// allocation failure or the tree's removal from the object database between the
    /// two reads can still reach the defect.
    ///
    /// Wraps: `git_note_commit_iterator_new`
    ///
    /// # Errors
    ///
    /// Fails with libgit2's error when the commit's tree is missing from the object database, or
    /// when it runs out of memory.
    #[expect(
        clippy::missing_panics_doc,
        reason = "cannot panic: successful note iterator is non-null"
    )]
    pub fn from_commit(commit: CommitRef<'_, 'r>) -> Result<NoteIteratorBox<'r>, RepositoryError> {
        let tree = commit.tree()?;
        let mut out = ptr::null_mut();
        // SAFETY: commit is live for the call and its repository for 'r; out is a
        // writable null slot. The C constructor retains its own root-tree reference
        // on success and releases every temporary on failure. Its tree lookup finds
        // `tree`, held until it returns, in the cache or rereads it, so it fails
        // only in the C defect documented above.
        let rc =
            unsafe { ffi::git_note_commit_iterator_new(&raw mut out, commit.as_ptr().cast_mut()) };
        drop(tree);
        if rc < 0 {
            return Err(RepositoryError::from_libgit2(rc));
        }
        // SAFETY: success transfers one initialized allocation paired with this
        // policy, with its repository kept alive by 'r.
        Ok(unsafe { NoteIteratorBox::from_c(out) }.expect("successful note iterator is non-null"))
    }
}

#[cfg(test)]
mod unit_tests {
    use std::path::Path;

    use super::*;
    use crate::libgit2::object::blob;
    use crate::libgit2::oid::oid::OidType;
    use crate::libgit2::repository::repository::Repository;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("crustify-notes-{tag}-{}", std::process::id()));
        drop(std::fs::remove_dir_all(&path));
        path
    }

    fn open(path: &Path) -> crate::libgit2::repository::repository::RepositoryBox {
        Repository::open(&CString::new(path.to_str().unwrap()).unwrap()).unwrap()
    }

    fn write_ref(path: &Path, name: &str, id: &Oid) {
        let file = path.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, id.as_ref().to_hex_string().unwrap() + "\n").unwrap();
    }

    /// C frees an uninitialised tree pointer when the notes commit's tree is missing;
    /// both constructors refuse in Rust with the tree lookup's error instead.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn missing_tree_returns_error_without_invalid_free() {
        let path = scratch("missing-tree");
        let repo = Repository::init(&CString::new(path.to_str().unwrap()).unwrap(), true).unwrap();
        // Store an intentionally incomplete object graph: no safe commit builder
        // accepts a missing tree, so use the raw object writer only for this fixture.
        let content = b"tree 1111111111111111111111111111111111111111\nauthor A <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\n\nnotes\n";
        let odb = repo.as_ref().odb().unwrap();
        let mut raw_id = ffi::git_oid::default();
        // SAFETY: odb is live, content readable for its length, and raw_id writable.
        // The ODB accepts arbitrary bytes; the commit parser validates them below.
        let rc = unsafe {
            ffi::git_odb_write(
                &raw mut raw_id,
                odb.as_c_ptr(),
                content.as_ptr().cast(),
                content.len(),
                ffi::GIT_OBJECT_COMMIT,
            )
        };
        assert_eq!(rc, 0);
        drop(odb);
        let id = Oid::from_raw(&raw_id.id[..20], OidType::Sha1).unwrap();
        let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
        for _ in 0..3 {
            let error = NoteIterator::from_commit(commit.as_ref()).err().unwrap();
            assert_eq!(error.code(), Some(ErrorCode::NotFound));
        }
        write_ref(&path, "refs/notes/commits", &id);
        assert_eq!(
            NoteIterator::new(repo.as_ref(), None).err().unwrap().code(),
            Some(ErrorCode::NotFound)
        );
        drop(commit);
        drop(repo);
        std::fs::remove_dir_all(path).unwrap();
    }

    /// C ignores a failed notes-commit lookup and then frees an uninitialised tree
    /// pointer; [`NoteIterator::new`] returns the lookup's error. Also resolves the
    /// default name through `core.notesRef` and symbolic references.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn non_commit_targets_return_lookup_error() {
        let path = scratch("non-commit");
        drop(Repository::init(&CString::new(path.to_str().unwrap()).unwrap(), true).unwrap());
        let repo = open(&path);
        let blob = blob::create_from_buffer(repo.as_ref(), b"not a commit\n").unwrap();
        let empty_tree =
            Oid::from_string(c"4b825dc642cb6eb9a060e54bf8d69288fbee4904", OidType::Sha1).unwrap();
        let missing =
            Oid::from_string(c"1111111111111111111111111111111111111111", OidType::Sha1).unwrap();
        write_ref(&path, "refs/notes/blob", &blob);
        write_ref(&path, "refs/notes/tree", &empty_tree);
        write_ref(&path, "refs/notes/missing", &missing);
        std::fs::write(path.join("refs/notes/symbolic"), "ref: refs/notes/blob\n").unwrap();
        for name in [
            c"refs/notes/blob",
            c"refs/notes/tree",
            c"refs/notes/missing",
            c"refs/notes/symbolic",
        ] {
            for _ in 0..2 {
                let error = NoteIterator::new(repo.as_ref(), Some(name)).err().unwrap();
                assert_eq!(error.code(), Some(ErrorCode::NotFound), "{name:?}");
            }
        }
        drop(repo);
        let mut config = std::fs::read_to_string(path.join("config")).unwrap();
        config.push_str("[core]\n\tnotesRef = refs/notes/tree\n");
        std::fs::write(path.join("config"), config).unwrap();
        let repo = open(&path);
        assert_eq!(
            NoteIterator::new(repo.as_ref(), None).err().unwrap().code(),
            Some(ErrorCode::NotFound)
        );
        // An invalid default name fails at reference resolution, as in C.
        drop(repo);
        let mut config = std::fs::read_to_string(path.join("config")).unwrap();
        config.push_str("[core]\n\tnotesRef = \"refs/notes/bad..name\"\n");
        std::fs::write(path.join("config"), config).unwrap();
        let repo = open(&path);
        let error = NoteIterator::new(repo.as_ref(), None).err().unwrap();
        assert_eq!(error.code(), Some(ErrorCode::InvalidSpec));
        drop(repo);
        std::fs::remove_dir_all(path).unwrap();
    }

    /// `git_note_default_ref` (and so `git_note_iterator_new` without a name) frees an
    /// uninitialised config entry when the configuration file stops parsing after it
    /// was loaded; the default name's lookup fails cleanly instead, each time, and
    /// an explicit name never reads the configuration.
    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn unparsable_configuration_fails_the_default_name() {
        let path = scratch("bad-config");
        drop(Repository::init(&CString::new(path.to_str().unwrap()).unwrap(), true).unwrap());
        let repo = open(&path);
        assert_eq!(
            NoteIterator::new(repo.as_ref(), None).err().unwrap().code(),
            Some(ErrorCode::NotFound)
        );
        // Opened while the file parsed, its configuration not loaded yet.
        let fresh = open(&path);
        let mut config = std::fs::read_to_string(path.join("config")).unwrap();
        config.push_str("[core\n");
        std::fs::write(path.join("config"), config).unwrap();
        let error = NoteIterator::new(repo.as_ref(), None).err().unwrap();
        assert_eq!(error.code(), Some(ErrorCode::Generic));
        assert_eq!(
            error.class(),
            Some(crate::libgit2::util::errors::ErrorClass::Config)
        );
        let error = NoteIterator::new(fresh.as_ref(), None).err().unwrap();
        assert_eq!(
            error.class(),
            Some(crate::libgit2::util::errors::ErrorClass::Config)
        );
        assert_eq!(
            NoteIterator::new(repo.as_ref(), Some(c"refs/notes/commits"))
                .err()
                .unwrap()
                .code(),
            Some(ErrorCode::NotFound)
        );
        drop((repo, fresh));
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    #[cfg_attr(miri, ignore = "calls libgit2")]
    fn owner_raw_roundtrip_and_null_adoption() {
        // SAFETY: a null pointer denotes absence and transfers no allocation.
        assert!(unsafe { NoteIteratorBox::<'static>::from_c(ptr::null_mut()) }.is_none());
        let repo = Repository::open(c"c/tests/resources/testrepo.git").unwrap();
        let id =
            Oid::from_string(c"d07b0f9a8c89f1d9e74dc4fce6421dec5ef8a659", OidType::Sha1).unwrap();
        let commit = Commit::lookup(repo.as_ref(), id.as_ref()).unwrap();
        let owner = NoteIterator::from_commit(commit.as_ref()).unwrap();
        let raw = owner.into_c();
        // SAFETY: raw is the sole allocation just transferred out of owner; repo
        // is live throughout this test, including the adopted owner's destruction.
        let mut adopted = unsafe { NoteIteratorBox::from_c(raw) }.unwrap();
        assert_eq!(adopted.as_ref().as_ptr(), raw);
        assert_eq!(adopted.as_mut().as_mut_ptr(), raw);
    }
}
