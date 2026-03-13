<p align="center">
  <img src="assets/banner.png?v=2" alt="alertpaca" width="400">
</p>

# alertpaca

[![Rust](https://img.shields.io/badge/rust-stable-orange)](https://www.rust-lang.org/)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Built with ratatui](https://img.shields.io/badge/tui-ratatui-purple)](https://ratatui.rs/)
[![GitHub release](https://img.shields.io/github/v/release/fabioconcina/alertpaca)](https://github.com/fabioconcina/alertpaca/releases/latest)

**Server health checker** — run it, see what needs attention.

Single binary, zero config, one screen.

## Why alertpaca?

alertpaca doesn't show you what's happening — it tells you what's about to go wrong.

It predicts when disks will fill up, checks if backups actually ran, notices when services silently disappear, and warns before TLS certificates expire. One screen, no graphs, no config required.

<p align="center">
  <img src="assets/screenshot.png" alt="alertpaca screenshot" width="700">
</p>

## AI & Automation

alertpaca is built for AI agents and scripts, not just humans.

- **MCP server** — `alertpaca --mcp` exposes a `check_health` tool via [Model Context Protocol](https://modelcontextprotocol.io). Connect it to Claude Desktop, Claude Code, or any MCP client.
- **JSON output** — `alertpaca --json` returns structured results. Pipe to `jq`, feed to an LLM, or parse in scripts.
- **Exit codes** — `0` = healthy, `2` = something needs attention. Use in CI, cron, or agent workflows.

```sh
# Ask an LLM to summarize server health
alertpaca --json | llm "summarize health, flag anything urgent"

# Filter problems
alertpaca --json | jq '.[] | select(.status != "Ok")'

# Alert if anything is wrong
alertpaca --once || notify-send "server needs attention"
```

See [MCP server](#mcp-server---mcp) below for Claude Desktop configuration.

## Quick start

Download a prebuilt binary from [the latest release](https://github.com/fabioconcina/alertpaca/releases/latest):

```bash
# Linux (x86_64)
curl -Lo alertpaca https://github.com/fabioconcina/alertpaca/releases/latest/download/alertpaca-linux-amd64
chmod +x alertpaca
./alertpaca

# macOS (Apple Silicon)
curl -Lo alertpaca https://github.com/fabioconcina/alertpaca/releases/latest/download/alertpaca-darwin-arm64
chmod +x alertpaca
./alertpaca
```

Or build from source:

```bash
cargo build --release
./target/release/alertpaca
```

Press `q` to quit, `r` to refresh, `↑↓` to scroll.

## CLI modes

### Interactive TUI (default)

```sh
alertpaca [-c path/to/config.toml]
```

Full-screen terminal UI with auto-refresh (60s), keyboard navigation, and a splash screen on startup.

| Key | Action |
|-----|--------|
| `q` | Quit |
| `r` | Force immediate refresh |
| `↑` / `k` | Scroll up |
| `↓` / `j` | Scroll down |

### JSON output (`--json`)

```sh
alertpaca --json
```

Runs all checks once, prints a JSON array to stdout, and exits. Suitable for piping to `jq`, scripts, or AI agents.

```json
[
  {
    "section": "System",
    "name": "cpu",
    "status": "Ok",
    "summary": "12% usage (4 cores)"
  },
  {
    "section": "Certificates",
    "name": "nextcloud.home.lan:443",
    "status": "Warning",
    "summary": "expires in 21d"
  }
]
```

### Plain-text table (`--once`)

```sh
alertpaca --once
```

Runs all checks once, prints a human-readable table to stdout, and exits.

### MCP server (`--mcp`)

```sh
alertpaca --mcp
```

Runs an [MCP](https://modelcontextprotocol.io) (Model Context Protocol) server on stdio, exposing a `check_health` tool. This allows AI agents (e.g. Claude Desktop, Claude Code) to check server health programmatically.

**Tool:** `check_health`
- **Parameters:** none
- **Returns:** JSON array of check results (same schema as `--json` output)

**Claude Desktop configuration:**

```json
{
  "mcpServers": {
    "alertpaca": {
      "command": "/path/to/alertpaca",
      "args": ["--mcp"]
    }
  }
}
```

### Exit codes

Exit codes apply to `--json` and `--once` modes:

| Code | Meaning |
|------|---------|
| 0 | All checks passed (Ok or Skipped) |
| 1 | Error (config load failure, I/O error) |
| 2 | At least one check returned Warning or Critical |

## What it checks

| Check | Auto-detected | Status |
|-------|:---:|--------|
| CPU usage | ✓ | warn >80%, critical >95% |
| Memory usage | ✓ | warn >80%, critical >95% |
| Swap usage | ✓ | warn >50%, critical >90% |
| Disk usage | ✓ | warn >80%, critical >90% |
| Disk fill prediction | ✓ | estimates days until full |
| System load | ✓ | warn > cores, critical > 2x cores |
| Uptime | ✓ | informational |
| Systemd failed units | ✓ | critical if any failed (configurable ignore list) |
| Docker containers | ✓ | warn if unhealthy/restarting |
| Backup freshness | config | warn at max_age, critical at 2x |
| TLS certificate expiry | config | warn <30d, critical <7d |
| Port/service drift | ✓ | warn if listeners disappear |
| NTP clock skew | ✓ | warn >500ms, critical >1s |
| HTTP endpoints | config | critical if unreachable/5xx, warn if 4xx |
| DNS resolution | config | critical if resolution fails, warn if >1s |
| Pending updates | ✓ | shows upgradable packages, warns on security updates |

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

[[endpoint]]
name = "Pi-hole"
url = "http://localhost:80"

[[endpoint]]
name = "Immich"
url = "http://localhost:2283"
# expect_status = 200  # optional, default: any 2xx/3xx is Ok

[[dns]]
name = "Pi-hole"
domain = "google.com"
server = "127.0.0.1"       # optional, defaults to 127.0.0.1

# Notifications — alert on status changes (ntfy.sh, Slack, Discord, etc.)
[notify]
url = "https://ntfy.sh/your-topic-here"

# Optional — ignore noisy systemd units
[systemd]
ignore = ["systemd-networkd-wait-online.service"]

# Optional — defaults to pool.ntp.org, 500ms warn, 1000ms critical
[ntp]
server = "pool.ntp.org"
# warn_ms = 500
# critical_ms = 1000
```

## Notifications

When `[notify]` is configured, alertpaca sends a POST to the URL whenever a check changes status. Alerts fire when checks transition to Warning/Critical, and recovery messages fire when they return to Ok. No notifications are sent when status stays the same.

Works with [ntfy.sh](https://ntfy.sh), Slack incoming webhooks, Discord webhooks, Gotify, or any endpoint that accepts a POST with a text body.

## State files

Stored in `~/.local/share/alertpaca/`:

- `history.json` — disk usage history for fill prediction
- `ports.json` — last known listening ports for drift detection
- `last_status.json` — previous check statuses for notification diffing

## License

MIT
