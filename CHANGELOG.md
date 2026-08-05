# Changelog

## v0.2.0 - 2026-08-06

- Send files or clipboard text to multiple LocalSend devices in one operation.
- Show live byte progress for outgoing and incoming transfers.
- Persist outgoing history metadata in SQLite without storing clipboard contents.
- Store multi-file history atomically, remove legacy clipboard payloads during migration, and clean stale temporary uploads at startup.
- Coalesce repeated discovery scans and handle successful empty API responses.
- Improve phone, tablet, landscape, dark-mode, and reduced-motion layouts.

## v0.1.0 - 2026-08-05

- Initial Docker-first LocalSend Web node release.
