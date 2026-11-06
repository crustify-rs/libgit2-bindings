//! Reads the committer from the environment with an ISO GIT_COMMITTER_DATE through the
//! safe API alone (the environment is set by the caller).
#![forbid(unsafe_code)]
use libgit2::libgit2::core::types::Signature;
use libgit2::libgit2::repository::repository::Repository;

fn main() {
    let dir = std::env::temp_dir().join(format!("envdate-probe-{}", std::process::id()));
    let path = std::ffi::CString::new(dir.to_str().unwrap()).unwrap();
    let repo = Repository::init(&path, true).unwrap();
    let c = Signature::default_committer_from_env(repo.as_ref()).unwrap();
    println!("{:?} {}", c.as_ref().name(), c.as_ref().when().seconds());
}
