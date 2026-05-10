default: run

build:
    cargo build

run:
    cargo run

build-release:
    cargo build --release

check:
    cargo clippy -- -D warnings
    cargo test

clean:
    cargo clean

# Tag the version in Cargo.toml and publish a GitHub release
# with linux-amd64 and linux-arm64 binaries. Requires zig + cargo-zigbuild.
# Opens $EDITOR for release notes.
release:
    #!/usr/bin/env bash
    set -euo pipefail

    if [ -n "$(git status --porcelain)" ]; then
        echo "working tree dirty, commit first" >&2
        exit 1
    fi

    VERSION=$(grep '^version' Cargo.toml | head -1 | cut -d'"' -f2)
    TAG="v${VERSION}"

    if git rev-parse "$TAG" >/dev/null 2>&1; then
        echo "tag $TAG already exists" >&2
        exit 1
    fi

    echo "Releasing $TAG"
    cargo zigbuild --release --target x86_64-unknown-linux-gnu
    cargo zigbuild --release --target aarch64-unknown-linux-gnu

    STAGE=$(mktemp -d)
    trap 'rm -rf "$STAGE"' EXIT
    cp target/x86_64-unknown-linux-gnu/release/alertpaca "$STAGE/alertpaca-linux-amd64"
    cp target/aarch64-unknown-linux-gnu/release/alertpaca "$STAGE/alertpaca-linux-arm64"

    git tag "$TAG"
    git push --tags

    gh release create "$TAG" --title "$TAG" \
        "$STAGE/alertpaca-linux-amd64" \
        "$STAGE/alertpaca-linux-arm64"
