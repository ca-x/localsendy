# Localsendy

[![CI](https://github.com/ca-x/localsendy/actions/workflows/ci.yml/badge.svg)](https://github.com/ca-x/localsendy/actions/workflows/ci.yml)
[![Container](https://github.com/ca-x/localsendy/actions/workflows/docker.yml/badge.svg)](https://github.com/ca-x/localsendy/actions/workflows/docker.yml)

Localsendy is a Docker-first LocalSend node with a responsive web interface. The Rust service handles LocalSend v2 discovery, encrypted receiving, transfer approval, sending, and storage; the browser remains a small control surface for any device on your LAN.

[简体中文](README.zh-CN.md)

## What works

- LocalSend v2 HTTPS receiver powered by [`localsend-rs`](https://github.com/CrossCopy/localsend-rs)
- Automatic multi-interface IPv4/IPv6 UDP multicast discovery with optional advanced filtering
- Multi-file sending to discovered LocalSend devices
- Explicit accept/decline flow or opt-in automatic acceptance
- Receive history and outgoing transfer status
- Responsive Send / Receive / Settings navigation based on the official LocalSend information architecture
- English, Simplified Chinese, and Traditional Chinese UI foundations
- Light, dark, and system themes with reduced-motion and keyboard support
- Non-root multi-stage Docker image and GHCR publishing workflow

## Quick start

Linux host networking is recommended because LocalSend discovery uses multicast UDP.

```bash
docker compose up -d
```

Open `http://<server-ip>:8080`. Received files are written to `./data/downloads`.

To use a different name:

```bash
LOCALSENDY_ALIAS="Home NAS" docker compose up -d
```

> Docker Desktop host networking support varies by version and platform. On Linux, `network_mode: host` gives the most reliable discovery and inbound transfer behavior.

LocalSend multicast discovery stays inside the same broadcast domain. On routed or cross-subnet networks, use the manual IP probe in the Send screen or configure a multicast relay.

With Linux host networking, Localsendy automatically monitors every eligible host interface; there is nothing to select for normal use. Newly available Ethernet, Wi-Fi, bridge, VPN, and tunnel interfaces are detected at runtime. Advanced settings can restrict discovery to specific interfaces and attach human-readable labels. Discovery supports the LocalSend IPv4 group `224.0.0.167` and IPv6 group `ff12::fd3a:e420`, including ULA networks such as `fc00::/7` when the network carries IPv6 multicast.

Network preferences and interface labels are saved to `/data/network-settings.json`. `LOCALSENDY_NETWORK_INTERFACES` is used as the initial fallback when no persisted settings exist.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `LOCALSENDY_BIND` | `0.0.0.0:8080` | Web UI and control API bind address |
| `LOCALSENDY_ALIAS` | `Localsendy` | Device name shown to LocalSend peers |
| `LOCALSENDY_PORT` | `53317` | LocalSend HTTPS and discovery port |
| `LOCALSENDY_DATA_DIR` | `/data` | Persistent storage root |
| `LOCALSENDY_DOWNLOAD_DIR` | `/data/downloads` | Directory for received files; overrides the data-root default |
| `LOCALSENDY_AUTO_ACCEPT` | `false` | Accept inbound transfers without browser approval |
| `LOCALSENDY_DISCOVERY_INTERVAL_SECONDS` | `30` | Presence announcement interval, minimum 5 seconds |
| `LOCALSENDY_NETWORK_INTERFACES` | `all` | Initial fallback: `all`, `*`, or a comma-separated interface list; persisted UI settings take precedence |
| `LOCALSENDY_MAX_UPLOAD_BYTES` | `10737418240` | Maximum total size of one browser send request |
| `RUST_LOG` | `localsendy=info,tower_http=info` | Rust tracing filter |

## Local development

Requirements: Rust 1.94+, Node.js 24+, npm 12+.

```bash
npm --prefix web ci
npm --prefix web run dev
```

In another terminal, build the frontend once and start Rust:

```bash
npm --prefix web run build
cargo run
```

Vite proxies `/api` to `127.0.0.1:8080` during development.

## Verification

```bash
npm --prefix web run typecheck
npm --prefix web run test:ci
npm --prefix web run build
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
docker build -t localsendy:dev .
```

## Architecture

```text
Browser (React/Vite)
        │ /api/v1
        ▼
Localsendy (Axum, :8080)
        ├── multicast discovery (UDP :53317)
        ├── LocalSend HTTPS receiver (TCP :53317)
        ├── LocalSend client for outgoing transfers
        └── /data/downloads + /data/tmp
```

See [docs/architecture.md](docs/architecture.md) and the persisted [design system](design-system/localsendy/MASTER.md).

## Roadmap

- Saved manual targets for networks where multicast is unavailable
- Text messages and share-by-link mode
- Transfer progress events and cancellation
- Persistent history with retention controls
- PIN-protected receiving and trusted-device policies
- Additional official LocalSend locale coverage

## Attribution

The product flow and interoperability target are informed by the official [LocalSend](https://github.com/localsend/localsend) project. Localsendy is an independent project and is not affiliated with or endorsed by LocalSend.

The protocol implementation is provided by the MIT-licensed [`localsend-rs`](https://github.com/CrossCopy/localsend-rs) crate.

## License

MIT
