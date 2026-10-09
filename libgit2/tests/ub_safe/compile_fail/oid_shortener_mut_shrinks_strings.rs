// The exclusive handle is invariant in the lifetime of the strings the shortener
// keeps: one over a `'static` shortener cannot accept shorter-lived strings.
use libgit2::libgit2::oid::oid::OidShortenerMut;

fn shrink<'a, 's>(m: OidShortenerMut<'a, 'static>) -> OidShortenerMut<'a, 's> {
    m
}

fn main() {}
