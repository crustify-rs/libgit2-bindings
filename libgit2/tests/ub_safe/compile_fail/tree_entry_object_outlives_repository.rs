// An entry's object is looked up in, and reads, the repository it was given, which it
// cannot outlive even though the entry can.
use libgit2::libgit2::object::object::ObjectArc;
use libgit2::libgit2::object::tree::TreeEntryBox;
use libgit2::libgit2::repository::repository::RepositoryBox;

fn escape(entry: &TreeEntryBox, repo: RepositoryBox) -> ObjectArc<'static> {
    entry.as_ref().to_object(repo.as_ref()).unwrap()
}

fn main() {
    let _ = escape;
}
