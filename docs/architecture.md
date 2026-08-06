# Architecture

Localsendy keeps the browser outside the LocalSend protocol boundary. Rust owns network identity, TLS, discovery, transfer sessions, and filesystem access; React talks only to the local control API.

## Runtime services

| Service | Bind | Responsibility |
| --- | --- | --- |
| Web/API | `0.0.0.0:52222` | Embedded React assets and `/api/v1` control endpoints |
| LocalSend receiver | `0.0.0.0:53317/tcp` | LocalSend v2 HTTPS register, prepare, upload, and cancel endpoints |
| Discovery | `0.0.0.0:53317/udp`, `[::]:53317/udp` | Automatic multi-interface LocalSend multicast presence and peer discovery |

The receiver creates an ephemeral self-signed certificate at startup and advertises its SHA-256 fingerprint as required by the LocalSend protocol.

Device identity is resolved at startup. Unless `LOCALSENDY_ALIAS` fixes the full alias, `/data/device-identity.json` stores language-neutral adjective and fruit indexes. This preserves one random identity while allowing `LOCALSENDY_ALIAS_LOCALE` or the Settings > Environment variables panel to render it in English, Simplified Chinese, or Traditional Chinese. The panel persists the display alias and random-name language, immediately announces a display-name change, and updates LocalSend HTTP identity responses without restarting the service. Type, model, and protocol port changes still require a service restart.

The browser control API deliberately assumes a trusted LAN and does not authenticate users. Operators must not expose port `52222` to untrusted networks. Enabling auto-accept also trusts every LocalSend peer that can reach the receiver to upload without a per-transfer confirmation.

## Data flow

### Outgoing

1. The browser streams selected files to `/api/v1/send` as multipart data.
2. Rust writes each part to a UUID-named temporary file under `/data/tmp` while enforcing the configured request limit.
3. The vendored official LocalSend Rust core prepares the remote upload and streams accepted files to the target.
4. The HTTP body stream updates per-target byte progress; temporary files are then removed and the transfer result is persisted.

### Incoming

1. A peer prepares an upload through the LocalSend HTTPS service.
2. The pending request is exposed to the browser through `/api/v1/pending`.
3. The user accepts or declines, unless `LOCALSENDY_AUTO_ACCEPT=true`.
4. The vendored official LocalSend Rust core reports written-byte progress while accepted data is stored beneath the configured data directory.

## Container networking

Multicast is the reason the default Compose file uses `network_mode: host`. A normal bridge network can publish TCP/UDP ports, but multicast discovery usually does not cross the bridge in the way LocalSend peers expect. In host mode, Localsendy automatically enumerates host interfaces, joins `224.0.0.167:53317` on each eligible IPv4 address and `ff12::fd3a:e420:53317` on each eligible IPv6 interface, and sends announcements through every bound socket. Interface changes are detected at runtime without user selection.

The `/api/v1/networks` settings are an optional advanced override for restricting discovery or assigning interface labels. They persist in `/data/network-settings.json`; `LOCALSENDY_NETWORK_INTERFACES` only supplies the initial fallback when that file does not exist. Multicast still stays inside each multicast domain unless the network provides a relay. Routed IPv6 unicast alone does not guarantee that a VPN or overlay forwards IPv6 multicast.

Automatic mode deduplicates only physical Ethernet/Wi-Fi adapters that cover the same non-link-local prefix, preferring Ethernet. The interface plan is rebuilt every five seconds, providing Wi-Fi failover without restart. Tunnel, bridge, and virtual interfaces are never collapsed by this rule.

## Current persistence boundaries

Files, settings, and transfer history persist under `/data`. SQLite stores outgoing batches atomically and restores the latest complete batches at startup; live progress remains in memory so active polling does not block on storage I/O. Clipboard contents are carried only in memory and are never written to transfer history.
