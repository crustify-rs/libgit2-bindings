#![forbid(unsafe_code)]
use std::ffi::CString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use libgit2::libgit2::src::libgit2::blob;
use libgit2::libgit2::src::libgit2::diff::{DiffDeltaRef, DiffFileRef};
use libgit2::libgit2::src::libgit2::repository::{Repository, RepositoryBox};
use libgit2::libgit2::src::libgit2::tree::{FileMode, Tree, TreeArc, TreeBuilder};

pub(super) const PATCH: &[u8] = b"diff --git a/file b/file\nindex 1111111..2222222 100644\n--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n";
pub(super) const RENAME: &[u8] =
    b"diff --git a/old b/new\nsimilarity index 100%\nrename from old\nrename to new\n";

pub(super) type FileSnapshot = (Option<Vec<u8>>, [u8; 32], u8, u64, u32, u16, u16);
pub(super) type DeltaSnapshot = (u32, u32, u16, u16, FileSnapshot, FileSnapshot);
pub(super) fn file(f: DiffFileRef<'_>) -> FileSnapshot {
    (
        f.path().map(|p| p.to_bytes().to_vec()),
        f.id().id(),
        f.id().raw_kind(),
        f.size(),
        f.flags(),
        f.mode(),
        f.id_abbrev(),
    )
}
pub(super) fn delta(d: DiffDeltaRef<'_>) -> DeltaSnapshot {
    (
        d.status(),
        d.flags(),
        d.similarity(),
        d.nfiles(),
        file(d.old_file()),
        file(d.new_file()),
    )
}

pub(super) struct Repo {
    pub repo: Option<RepositoryBox>,
    pub path: PathBuf,
}
impl Repo {
    pub(super) fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "crustify-diff-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        drop(std::fs::remove_dir_all(&path));
        let repo = Repository::init(&CString::new(path.to_str().unwrap()).unwrap(), false).unwrap();
        Self {
            repo: Some(repo),
            path,
        }
    }

    pub(super) fn get(&self) -> &RepositoryBox {
        self.repo.as_ref().unwrap()
    }

    pub(super) fn tree(&self, text: &[u8]) -> TreeArc<'_> {
        let repo = self.get().as_ref();
        let oid = blob::create_from_buffer(repo, text).unwrap();
        let mut builder = TreeBuilder::new(repo).unwrap();
        builder
            .as_mut()
            .insert(c"file", oid.as_ref(), FileMode::Blob)
            .unwrap();
        let id = builder.as_mut().write().unwrap();
        Tree::lookup(repo, id.as_ref()).unwrap()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        drop(self.repo.take());
        drop(std::fs::remove_dir_all(&self.path));
    }
}
