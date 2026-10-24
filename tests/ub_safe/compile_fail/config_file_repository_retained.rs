use {libgit2::libgit2::src::libgit2::config::Config, libgit2::libgit2::src::libgit2::config::ConfigBox, libgit2::libgit2::src::libgit2::config::ConfigLevel};
use {libgit2::libgit2::src::libgit2::repository::Repository, libgit2::libgit2::src::libgit2::repository::RepositoryRef};
fn bind<'r>(repo: RepositoryRef<'r>) -> ConfigBox<'r> {
    let mut cfg: ConfigBox<'r> = Config::new().unwrap();
    cfg.as_mut().add_file(c"config", ConfigLevel::LOCAL, Some(repo), false).unwrap();
    cfg
}
fn main() {
    let repo = Repository::open(c".").unwrap();
    let cfg = bind(repo.as_ref());
    drop(repo);
    drop(cfg);
}
