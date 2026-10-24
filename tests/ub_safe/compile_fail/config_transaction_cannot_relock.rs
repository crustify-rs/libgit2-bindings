use libgit2::libgit2::src::libgit2::config::Config;
fn main() {
    let mut config = Config::new().unwrap();
    let mut tx = config.as_mut().lock().unwrap();
    let nested = tx.config_mut().lock();
    drop(nested);
}
