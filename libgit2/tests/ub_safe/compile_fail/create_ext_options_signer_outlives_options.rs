// The options borrow their signing callback exclusively for as long as they live: it
// cannot be dropped (nor used) while they may still call it.
use libgit2::libgit2::core::types::StreamError;
use libgit2::libgit2::object::commit::{CommitBuilderMut, CreateExtOptions, SignatureCallback};
use libgit2::libgit2::repository::repository::RepositoryRef;

fn main() {
    let mut f = |_: CommitBuilderMut<'_>, _: RepositoryRef<'_>, _: &core::ffi::CStr| {
        Ok::<(), StreamError>(())
    };
    let mut signer = SignatureCallback::new(&mut f);
    let mut options = CreateExtOptions::new();
    options.as_mut().set_signer(Some(&mut signer));
    drop(signer);
    let _ = options.as_ref().has_signer();
}
