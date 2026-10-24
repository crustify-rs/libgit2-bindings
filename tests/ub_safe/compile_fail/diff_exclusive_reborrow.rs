use libgit2::libgit2::src::libgit2::diff::Diff;
fn main() {
    let mut d = Diff::from_buffer(b"").unwrap();
    let mut m = d.as_mut();
    let r = m.as_ref();
    let _exclusive = m.as_mut();
    let _ = r.len();
}
