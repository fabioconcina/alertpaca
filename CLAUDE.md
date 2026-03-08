# alertpaca

Proactive server health checker TUI. Rust, single binary.

## Build

```
cargo build          # debug
cargo build --release  # release (stripped, LTO)
```

## Run

```
cargo run
cargo run -- --config path/to/config.toml
```

## Test

```
cargo test
cargo clippy -- -D warnings
```

## Architecture

- Fully synchronous (no tokio)
- Background thread for checks, mpsc channel to TUI
- Each check module returns `Vec<CheckResult>`, never panics
- State files in `~/.local/share/alertpaca/`
- Config in `~/.config/alertpaca/config.toml`

## Conventions

- Keep dependencies minimal
- No openssl — use rustls
- Graceful degradation: if a check fails, show Skipped status
- Linux-specific features gated with `#[cfg(target_os = "linux")]`

## Documentation

When adding or changing features, **always update both**:

- `README.md` — user-facing documentation with examples
- `llms.txt` — LLM-friendly plain-text reference (same content, no markdown images/badges)
