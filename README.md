<p align="center">
  <img src="assets/banner.png?v=2" alt="alertpaca" width="400">
</p>

# alertpaca

[![Rust](https://img.shields.io/badge/rust-stable-orange)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Built with ratatui](https://img.shields.io/badge/tui-ratatui-purple)](https://ratatui.rs/)

**Server health checker** — run it, see what needs attention.

Single binary, zero config, one screen.

## Why alertpaca?

alertpaca doesn't show you what's happening — it tells you what's about to go wrong.

It predicts when disks will fill up, checks if backups actually ran, notices when services silently disappear, and warns before TLS certificates expire. One screen, no graphs, no config required.

<p align="center">
  <img src="assets/screenshot.png" alt="alertpaca screenshot" width="700">
</p>

## Quick start

```bash
# Build from source
cargo build --release

# Run — zero config needed
./target/release/alertpaca
```

Press `q` to quit, `r` to refresh, `↑↓` to scroll.

## What it checks

| Check | Auto-detected | Status |
|-------|:---:|--------|
| CPU usage | ✓ | warn >80%, critical >95% |
| Memory usage | ✓ | warn >80%, critical >95% |
| Disk usage | ✓ | warn >80%, critical >90% |
| Disk fill prediction | ✓ | estimates days until full |
| System load | ✓ | warn > cores, critical > 2x cores |
| Uptime | ✓ | informational |
| Systemd failed units | ✓ | critical if any failed |
| Docker containers | ✓ | warn if unhealthy/restarting |
| Backup freshness | config | warn at max_age, critical at 2x |
| TLS certificate expiry | config | warn <30d, critical <7d |
| Port/service drift | ✓ | warn if listeners disappear |

## Configuration

Optional. Create `~/.config/alertpaca/config.toml`:

```toml
[[backup]]
name = "documents"
type = "file"
path = "/mnt/backup/docs"
pattern = "backup-*.tar.gz"
max_age = "24h"

[[backup]]
name = "photos"
type = "restic"
repo = "/mnt/backup/restic-photos"
max_age = "7d"
# password_file = "/etc/restic/password"

[[backup]]
name = "tank/data"
type = "zfs"
dataset = "tank/data"
max_age = "1h"

[[certificate]]
endpoint = "nextcloud.home.lan:443"

[[certificate]]
endpoint = "jellyfin.home.lan:443"
```

## State files

Stored in `~/.local/share/alertpaca/`:

- `history.json` — disk usage history for fill prediction
- `ports.json` — last known listening ports for drift detection

## License

MIT
