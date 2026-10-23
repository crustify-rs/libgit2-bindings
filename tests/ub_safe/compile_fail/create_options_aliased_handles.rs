// A shared handle cannot coexist with the exclusive one of the same options, nor a
// signature or a run of headers it lent with a change of them.
use libgit2::libgit2::core::types::{Signature, Time};
use libgit2::libgit2::object::commit::{CreateOptions, Header};

fn main() {
    let mut options = CreateOptions::new();
    let shared = options.as_ref();
    let mut exclusive = options.as_mut();
    exclusive.set_allow_empty_commit(true);
    let _ = shared.allow_empty_commit();

    let mut headers = [Header::new(c"f", c"v")];
    let mut options = CreateOptions::new();
    options.as_mut().set_extra_headers(&headers);
    headers[0].as_mut().set_value(c"w");
    let _ = options.as_ref().extra_headers();

    let mut author = Signature::new(c"A", c"a@example.com", 0, 0).unwrap();
    let mut options = CreateOptions::new();
    options.as_mut().set_author(Some(author.as_ref()));
    author.as_mut().set_when(Time::new(1, 0));
    let _ = options.as_ref().author();
}
