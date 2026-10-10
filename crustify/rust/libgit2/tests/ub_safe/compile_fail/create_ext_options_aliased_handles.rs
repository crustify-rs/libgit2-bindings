// A shared handle cannot coexist with the exclusive one of the same options, nor a
// run of headers it lent with a change of the headers.
use libgit2::libgit2::object::commit::{CreateExtOptions, Header};

fn main() {
    let mut options = CreateExtOptions::new();
    let shared = options.as_ref();
    let mut exclusive = options.as_mut();
    exclusive.set_update_ref(Some(c"HEAD"));
    let _ = shared.update_ref();

    let mut headers = [Header::new(c"f", c"v")];
    let mut options = CreateExtOptions::new();
    options.as_mut().set_extra_headers(&headers);
    headers[0].as_mut().set_value(c"w");
    let _ = options.as_ref().extra_headers();
}
