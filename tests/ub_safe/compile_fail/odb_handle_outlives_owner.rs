// A handle lent by an `OdbArc` cannot outlive it: dropping the owner may free the odb.
use libgit2::libgit2::odb::odb::Odb;

fn main() {
    let odb = Odb::new().unwrap();
    let handle = odb.as_ref();
    drop(odb);
    let _ = format!("{handle:?}");
}
