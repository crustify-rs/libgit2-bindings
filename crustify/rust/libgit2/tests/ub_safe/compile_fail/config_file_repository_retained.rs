use libgit2::libgit2::config::config::{Config, ConfigBox, ConfigLevel};
use libgit2::libgit2::repository::repository::{Repository, RepositoryRef};
fn bind<'r>(repo: RepositoryRef<'r>) -> ConfigBox<'r> {
    let mut cfg: ConfigBox<'r> = Config::new().unwrap();
    cfg.as_mut().add_file_ondisk(c"config", ConfigLevel::LOCAL, Some(repo), false).unwrap();
    cfg
}
fn main() {
    let repo = Repository::open(c".").unwrap();
    let cfg = bind(repo.as_ref());
    drop(repo);
    drop(cfg);
}
