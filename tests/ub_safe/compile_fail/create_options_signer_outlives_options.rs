// The options borrow their signing callback exclusively for as long as they live: it
// cannot be dropped (nor used) while they may still call it.
use libgit2::libgit2::src::libgit2::filter::StreamError;
use {libgit2::libgit2::src::libgit2::commit::CommitBuilderMut, libgit2::libgit2::src::libgit2::commit::CreateOptions, libgit2::libgit2::src::libgit2::commit::SignatureCallback};
use libgit2::libgit2::src::libgit2::repository::RepositoryRef;

fn main() {
    let mut f = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &core::ffi::CStr| {
        Ok::<(), StreamError>(())
    };
    let mut signer = SignatureCallback::new(&mut f);
    let mut options = CreateOptions::new();
    options.as_mut().set_signer(Some(&mut signer));
    signer.resume_panic();
    let _ = options.as_ref().has_signer();
}
