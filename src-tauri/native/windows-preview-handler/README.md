# Windows Preview Handler v2 spike

This crate is the isolated W4-03 bounded-capture spike. `IInitializeWithStream`
retains one owner-STA `IStream` reference and performs no content work. The
owner STA synchronously captures at most 512 KiB in `DoPreview`, drops the
shell stream, and only then registers an immutable memory-backed HostProvided
source and admits deferred Text/Code/Markdown representation work.

Deferred work owns bounded bytes, a request/generation token, cancellation
state, and completion data only. It never receives an `IStream`, COM proxy or
clone, file handle, path, PreviewSession, read gate, scheduler or renderer
authority. The HostProvided implementation is shared with the app through
`../host-provided`; representation logic is shared through
`../preview-representation`.

The harness uses the test CLSID and test-only observation exports. It does not
write registry state or product associations. The harness also retains a
separate standard-marshaled non-cooperative COM negative regression; that
diagnostic uses `CoCancelCall` only to demonstrate why the production handler
does not use cancellation as its source-release guarantee.

Build and run on Windows with:

```text
cargo build --release --manifest-path src-tauri/native/windows-preview-handler/Cargo.toml --features test-observability
cargo build --release --manifest-path src-tauri/native/windows-preview-handler-harness/Cargo.toml
fylura-windows-preview-handler-harness.exe <preview-handler.dll> <dedicated-fixture-root>
```

Real Explorer/prevhost registration and evidence remain outside this
controlled harness and must be performed separately before W4-03 v2 can be
declared complete.
