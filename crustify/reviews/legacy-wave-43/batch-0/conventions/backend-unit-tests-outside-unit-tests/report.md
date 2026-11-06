# Backend unit tests lived in two extra top-level test modules

- **item**: `git_config_backend` (type), `crustify/rust/libgit2/libgit2/config/sys/config.rs`
- **affected commit**: `e797d4624` (landed by `beeda6a07`)
- **class**: conventions

## Finding

The prompt's test layout puts inline tests in one `#[cfg(test)] mod unit_tests` beside
the translated unit. The batch added `#[cfg(test)] mod backend_unit_tests` and
`#[cfg(test)] mod backend_input_unit_tests` after it. This TU already nests per-type
groups inside `unit_tests` (`backend_entries`, `iterators`), and wave 3's review moved
the iterator tests there for the same reason
(`wave-3/batch-0/conventions/iterator-unit-tests-outside-unit-tests`).

## Fix

Both modules are now one `unit_tests::backends` group, with the input-rejection test
folded in. The bodies are unchanged except where this review's other fixes touch
them. The group's helpers call the error setter as `errors::set`, because the mock's
own `set` callback shadows the glob-imported name. Every test still runs, under its
new path `unit_tests::backends::*`.
