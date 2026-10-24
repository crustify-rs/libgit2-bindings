// The routines committing on `HEAD` borrow their options exclusively for the call: no
// shared handle to them may be held across it (their signer is lent the commit being
// written, and libgit2 reads every field in place).
use {libgit2::libgit2::src::libgit2::commit::CreateOptions, libgit2::libgit2::src::libgit2::commit::create_from_stage};
use libgit2::libgit2::src::libgit2::repository::RepositoryRef;

fn commit(repo: RepositoryRef<'_>) {
    let mut options = CreateOptions::new();
    let shared = options.as_ref();
    let _ = create_from_stage(repo, c"m\n", Some(options.as_mut()));
    let _ = shared.allow_empty_commit();
}

fn main() {}
