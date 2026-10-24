// An exclusive handle of a `'static` trailer stores only `'static` strings: it is
// invariant in the strings' lifetime, so a shorter-lived one does not fit.
use libgit2::libgit2::src::libgit2::trailer::MessageTrailer;
use std::ffi::CString;

fn main() {
    let mut trailer: MessageTrailer<'static> = MessageTrailer::new(c"Fixes", c"#1");
    {
        let local = CString::new("#2").unwrap();
        trailer.as_mut().set_value(&local);
    }
    let _: MessageTrailer<'static> = trailer;
}
