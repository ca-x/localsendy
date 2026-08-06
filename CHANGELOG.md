# Changelog

## v0.3.0 - 2026-08-06

- Add official-style link sharing for selected files on the existing web port at `/share`.
- Keep one active share at a time, allow repeated downloads while it is active, and clean staged files when it is replaced or stopped.
- Add per-browser approval, decline, automatic acceptance, PIN protection, and PIN attempt limiting.
- Add share management with responsive request, file, QR code, and access-control views in all supported locales.
- Reuse LocalSend's official browser download page and session semantics while keeping the normal LocalSend HTTPS receiver running.

## v0.2.1 - 2026-08-06

- Add a Settings > Environment variables panel for runtime automatic acceptance.
- Allow editing the LocalSend display name and regenerating stable multilingual random names.
- Apply display-name and automatic-acceptance changes immediately, persist them in SQLite, and rebroadcast the updated identity.
- Confirm the trusted-network implication before enabling automatic acceptance and document the unauthenticated control API boundary.
- Polish the new settings form for keyboard access, mobile touch targets, loading feedback, and all supported locales.

## v0.2.0 - 2026-08-06

- Send files or clipboard text to multiple LocalSend devices in one operation.
- Show live byte progress for outgoing and incoming transfers.
- Persist outgoing history metadata in SQLite without storing clipboard contents.
- Store multi-file history atomically, remove legacy clipboard payloads during migration, and clean stale temporary uploads at startup.
- Coalesce repeated discovery scans and handle successful empty API responses.
- Improve phone, tablet, landscape, dark-mode, and reduced-motion layouts.

## v0.1.0 - 2026-08-05

- Initial Docker-first LocalSend Web node release.
