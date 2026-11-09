# Temporary rusqlite compatibility patch

Source: published MIT-licensed `r2d2_sqlite` 0.34.0 from crates.io, the dependency already used by Fylura. `src/lib.rs` is unmodified; the MIT license text is preserved with LF line endings. Both upstream manifest rusqlite requirements change from `0.39` to `0.41`; the upstream init test uses the replacement `trace_v2` API because rusqlite 0.41 removed `trace`.

Why: RUSTSEC-2026-0459 / CVE-2026-119375 affects bundled SQLite before 3.54.0. Upgrading to rusqlite 0.41 / libsqlite3-sys 0.39 fixes it. The latest released r2d2_sqlite 0.35 still requires rusqlite 0.40, so Cargo's native `links=sqlite3` constraint prevents simply upgrading the app while using the registry pool adapter.

Removal condition: when upstream releases an adapter accepting rusqlite 0.41 or a newer patched compatible chain, remove this `[patch.crates-io]`, update the existing dependency and regenerate the lockfile. This patch changes no connection management, pool limits, database schema, migration, journaling or recovery behavior. Preserve the upstream MIT license.

The app hook installer and its benchmark caller accept the mutable connection now required by rusqlite 0.41 hook registration. Hook bodies, admission policy and connection initialization are unchanged.
