// The options borrow their signatures, strings and headers for `'o`: none may go first.
use libgit2::libgit2::core::types::Signature;
use libgit2::libgit2::object::commit::{CreateOptions, Header};
use std::ffi::CString;

fn main() {
    let mut options = CreateOptions::new();
    {
        let author = Signature::new(c"A", c"a@example.com", 0, 0).unwrap();
        options.as_mut().set_author(Some(author.as_ref()));
    }
    let _ = options.as_ref().author();

    let mut options = CreateOptions::new();
    {
        let headers = vec![Header::new(c"f", c"v")];
        options.as_mut().set_extra_headers(&headers);
    }
    let _ = options.as_ref().extra_headers();

    let mut options = CreateOptions::new();
    {
        let encoding = CString::new("ISO-8859-1").unwrap();
        options.as_mut().set_message_encoding(Some(&encoding));
    }
    let _ = options.as_ref().message_encoding();

    // A signature handle read back cannot outlive what it was set from either.
    let committer;
    {
        let owner = Signature::new(c"C", c"c@example.com", 0, 0).unwrap();
        let mut options = CreateOptions::new();
        options.as_mut().set_committer(Some(owner.as_ref()));
        committer = options.as_ref().committer();
    }
    let _ = committer;
}
