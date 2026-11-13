# Local drag patch

This is the upstream `drag` 2.1.1 crate, vendored under its Apache-2.0/MIT
licenses. The only source change is in the macOS backend: file URLs now carry
the real directory flag so Finder receives dropped folders as folders.

The dependency and its upstream release remain declared in `Cargo.lock` and
`Cargo.toml`; update this copy deliberately when changing the upstream version.
