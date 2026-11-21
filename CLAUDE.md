# alertpaca

Proactive server health checker TUI. Rust, single binary.

## Build

```
cargo build          # debug
cargo build --release  # release (stripped, LTO)
just cross           # linux amd64 deploy build via cargo-zigbuild (needs zig)
```

## Run

```
cargo run
cargo run -- --config path/to/config.toml
```

## Test

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

CI (`.github/workflows/ci.yml`) runs the same three on every push and PR,
plus a `cargo check` on the MSRV (`rust-version` in Cargo.toml).

## Architecture

- Fully synchronous (no tokio)
- Background thread for checks, mpsc channel to TUI
- Each check module returns `Vec<CheckResult>`, never panics
- State files in `~/.local/share/alertpaca/`
- Config in `~/.config/alertpaca/config.toml`

## Version

Single source of truth: `Cargo.toml` (`version` field).

## Release

Releases are built and published by CI (`.github/workflows/release.yml`).
Never publish a release locally.

1. Bump `version` in `Cargo.toml` and run `cargo build` so `Cargo.lock` follows.
2. Write `RELEASE_NOTES.md` (it becomes the GitHub release body).
3. Commit and push to `master`.
4. Push the matching tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.

The workflow fails if the tag doesn't match the `Cargo.toml` version. It builds
`alertpaca-linux-amd64` and `alertpaca-linux-arm64` with cargo-zigbuild
(glibc 2.17 floor), then publishes them with `checksums.txt`, Syft SBOMs and
build-provenance attestations. Keep those asset names: README download links
depend on them.

## Conventions

- Keep dependencies minimal
- No openssl: use rustls
- Graceful degradation: if a check fails, show Skipped status
- Linux-specific features gated with `#[cfg(target_os = "linux")]`

## Documentation

When adding or changing features, **always update both**:

- `README.md`: user-facing documentation with examples
- `llms.txt`: LLM-friendly plain-text reference (same content, no markdown images/badges)
