default: run

build:
    cargo build

run:
    cargo run

build-release:
    cargo build --release

check:
    cargo fmt --check
    cargo clippy --all-targets -- -D warnings
    cargo test

clean:
    cargo clean

# Linux amd64 build for deploying to the home server (needs zig +
# cargo-zigbuild). Same glibc floor as the release binaries.
# Output: target/x86_64-unknown-linux-gnu/release/alertpaca
cross:
    cargo zigbuild --release --target x86_64-unknown-linux-gnu.2.17

# Releases are built and published by CI, never locally.
release:
    @echo "Releases are published by .github/workflows/release.yml:"
    @echo "  1. bump version in Cargo.toml (cargo build to refresh Cargo.lock)"
    @echo "  2. write RELEASE_NOTES.md"
    @echo "  3. commit and push to master"
    @echo "  4. git tag vX.Y.Z && git push origin vX.Y.Z"
