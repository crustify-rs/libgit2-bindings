// The options borrow their strings and headers for `'o`: none may go first, and the
// headers' strings are borrowed for as long.
use libgit2::libgit2::object::commit::{CreateExtOptions, Header};
use std::ffi::CString;

fn main() {
    let mut options = CreateExtOptions::new();
    {
        let headers = vec![Header::new(c"f", c"v")];
        options.as_mut().set_extra_headers(&headers);
    }
    let _ = options.as_ref().extra_headers();

    let mut options = CreateExtOptions::new();
    {
        let name = CString::new("refs/heads/main").unwrap();
        options.as_mut().set_update_ref(Some(&name));
    }
    let _ = options.as_ref().update_ref();

    let mut options = CreateExtOptions::new();
    let headers;
    {
        let value = CString::new("v").unwrap();
        headers = [Header::new(c"f", &value)];
    }
    options.as_mut().set_extra_headers(&headers);
}
