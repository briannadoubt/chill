# Chill Rust fuzz targets

These `cargo-fuzz` targets exercise attacker-controlled parsers and validators
without needing PostgreSQL or object storage:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cd backend/fuzz
cargo fuzz run credential_parser -- -max_total_time=60
cargo fuzz run ingest_candidate -- -max_total_time=60
cargo fuzz run query_plan -- -max_total_time=60
```

`cargo-fuzz` requires the nightly compiler. The fuzz workspace is intentionally
separate from the release workspace, so it cannot add nightly-only dependencies
or compiler behavior to production binaries.

Long-running CI or a scheduled security job should preserve each generated
`fuzz/artifacts/` reproducer and add non-sensitive regression inputs under
`fuzz/corpus/<target>/`. Do not check in payloads captured from production.
The release security review records the duration, commit, sanitizer, target,
corpus digest, and crash count; an empty harness is not a passing result.
