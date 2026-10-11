use libgit2::libgit2::config::config::Config;
fn main() {
    let mut config = Config::new().unwrap();
    let tx = config.as_mut().lock().unwrap();
    let _alias = config.as_mut();
    drop(config);
    drop(tx);
}
