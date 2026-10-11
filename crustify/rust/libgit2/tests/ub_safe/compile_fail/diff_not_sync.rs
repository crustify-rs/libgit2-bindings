use libgit2::libgit2::diff::diff::Diff;
fn sync<T: Sync>() {}
fn main() { sync::<Diff<'static>>(); }
