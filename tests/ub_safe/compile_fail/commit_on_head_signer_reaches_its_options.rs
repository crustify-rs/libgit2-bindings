// A signer cannot reach the options it is lent through: the closure would read them
// (through a `Cell` filled once the signer is stored) while the routine committing on
// `HEAD` borrows them exclusively.
use std::cell::Cell;

use libgit2::libgit2::src::libgit2::filter::StreamError;
use {libgit2::libgit2::src::libgit2::commit::CommitBuilderMut, libgit2::libgit2::src::libgit2::commit::CreateOptions, libgit2::libgit2::src::libgit2::commit::SignatureCallback, libgit2::libgit2::src::libgit2::commit::amend_from_stage};
use libgit2::libgit2::src::libgit2::repository::RepositoryRef;

fn commit(repo: RepositoryRef<'_>) {
    let reached: Cell<Option<&CreateOptions<'_>>> = Cell::new(None);
    let mut f = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &core::ffi::CStr| {
        if let Some(options) = reached.get() {
            let _ = options.as_ref().has_signer();
        }
        Ok::<(), StreamError>(())
    };
    let mut signer = SignatureCallback::new(&mut f);
    let mut options = CreateOptions::new();
    options.as_mut().set_signer(Some(&mut signer));
    reached.set(Some(&options));
    let _ = amend_from_stage(repo, None, Some(options.as_mut()));
}

fn main() {}
