default: run

build:
    cargo build

run:
    cargo run

release:
    cargo build --release

check:
    cargo clippy -- -D warnings
    cargo test

clean:
    cargo clean
