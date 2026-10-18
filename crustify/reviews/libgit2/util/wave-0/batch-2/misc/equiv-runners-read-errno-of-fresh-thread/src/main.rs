//! A freshly spawned thread can start with `errno == EAGAIN` while other threads are
//! being created and torn down: std's thread start waits on a contended lock through
//! `libc::syscall`, which leaves the futex's `EAGAIN` in the new thread's `errno`.
//!
//! Exits non-zero once a fresh thread reads a non-zero `errno` as its first action;
//! gives up after 20 seconds.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::thread;
use std::time::Instant;

fn errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

static STOP: AtomicBool = AtomicBool::new(false);
static HITS: AtomicUsize = AtomicUsize::new(0);

fn main() {
    let start = Instant::now();
    let churn: Vec<_> = (0..4)
        .map(|_| {
            thread::spawn(|| {
                while !STOP.load(Relaxed) {
                    let hs: Vec<_> = (0..8)
                        .map(|_| thread::spawn(|| std::hint::black_box(vec![1u8; 1 << 16])))
                        .collect();
                    for h in hs {
                        h.join().unwrap();
                    }
                }
            })
        })
        .collect();
    let mut probed = 0usize;
    while start.elapsed().as_secs() < 20 && HITS.load(Relaxed) == 0 {
        let hs: Vec<_> = (0..4)
            .map(|_| {
                thread::spawn(|| {
                    let e = errno();
                    if e != 0 {
                        HITS.fetch_add(1, Relaxed);
                        eprintln!("thread {:?} started with errno {e}", thread::current().id());
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
            probed += 1;
        }
    }
    STOP.store(true, Relaxed);
    for c in churn {
        c.join().unwrap();
    }
    let hits = HITS.load(Relaxed);
    println!(
        "{hits} of {probed} fresh threads started with a non-zero errno, in {:?}",
        start.elapsed()
    );
    std::process::exit(if hits > 0 { 1 } else { 0 });
}
