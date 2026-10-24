use libgit2::libgit2::src::libgit2::diff::Diff;
fn main() {
    let d = Diff::from_buffer(b"").unwrap();
    let file = d.as_ref().get(0).unwrap().old_file();
    drop(d);
    let _ = file.path();
}
