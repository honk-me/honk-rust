# Changelog

All notable changes to the `honk-me` crate are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-10-04

### Added
- Async `Honk` client for `POST /v1/messages` on Tokio and `reqwest` with rustls (no
  OpenSSL), with every field of the v1 ingestion API (including `image_url`):
  `Honk::new(url, key)`, `Honk::builder()`, `Honk::from_env()`.
- `send(&message, idempotency_key)` returning `Accepted { id, duplicate, received_at }`.
- Automatic UUIDv7 `Idempotency-Key` (or your own), reused on every retry.
- Retries for network errors, timeouts, 429 and 5xx with exponential backoff, full jitter,
  `Retry-After` and a total deadline; redirects are reported, never followed.
- The `Error` enum: `Validation`, `Auth`, `Quota`, `Conflict`, `Network`, `Timeout`, `Server`,
  `Http` (each with a `Failure`), `InvalidConfiguration`; `is_retryable()`.
- Local validation of limits, https-only URLs, metadata and the 16 KiB body, with every invalid
  field reported at once; `validate(false)` leaves it to the server.
- The Honk scale: `Severity::LIGHT`, `BEEP`, `LOUD`, `LONG`, `BLAST` (aliases of the canonical
  severities), case-insensitive parsing, always sent canonical; the `light`, `beep`, `loud`,
  `long`, `blast` helpers and the `info`, `success`, `warning`, `error`, `critical` synonyms;
  `problem` and `recovery`. Helpers return a `PendingSend` to chain fields and `.await`.
- `blocking` feature: `honk_me::blocking::Honk`, a synchronous client.
- `encode_message` (the exact JSON body, for dry runs) and `new_idempotency_key`.
