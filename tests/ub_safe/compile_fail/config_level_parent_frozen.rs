use {libgit2::libgit2::src::libgit2::config::Config, libgit2::libgit2::src::libgit2::config::ConfigLevel};
fn main() {
    let mut cfg = Config::open(c"config").unwrap();
    let mut parent = cfg.as_mut();
    let view = parent.open_level(ConfigLevel::LOCAL).unwrap();
    let _other = parent.lock();
    drop(view);
}
