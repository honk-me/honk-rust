# honk-me (Rust)

[![CI](https://github.com/honk-me/honk-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/honk-me/honk-rust/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/honk-me)](https://crates.io/crates/honk-me)
[![docs.rs](https://img.shields.io/docsrs/honk-me)](https://docs.rs/honk-me)

Official Rust client for [Honk](https://honk-me.app), the inbox that turns events from your
apps, jobs, scripts and CI into calm, grouped push notifications on your phone.

- Async-first on Tokio, `reqwest` with rustls (no OpenSSL). A synchronous client with the
  `blocking` feature.
- Retries with backoff, `Retry-After`, a total deadline and an idempotency key on every send,
  so a retry never creates a duplicate.
- Rust 1.85+. `#![forbid(unsafe_code)]`.

> **Keep the key on the server.** An ingestion key (`honk_…`) lets anyone post into your
> project. Use this crate in services, jobs, CLIs and CI only; never ship it inside a desktop,
> mobile or WebAssembly front end. Read it from the environment.

## Install

```sh
cargo add honk-me
cargo add tokio --features macros,rt-multi-thread   # if you don't have a runtime yet
```

or, for synchronous code, `cargo add honk-me --features blocking`.

Create a project and an ingestion key at [honk-me.app](https://honk-me.app). Its
*Integrations* page generates ready-to-paste code.

## Quick start

```rust
use honk_me::Honk;

#[tokio::main]
async fn main() -> honk_me::Result<()> {
    let honk = Honk::from_env()?; // HONK_URL, HONK_KEY (+ HONK_SOURCE, HONK_ENVIRONMENT, HONK_CHANNEL)
    honk.beep("Backup finished", "nightly pg_dump took 42 s").await?;
    Ok(())
}
```

Or explicitly: `Honk::new("https://honk-me.app", key)?`. A missing or malformed URL or key is
`Error::InvalidConfiguration`. `Honk` is cheap to clone: create one per application and share
it, so connections are kept alive.

## The Honk scale

Every severity has a horn name. Use either; the SDK always sends the canonical value.

| Horn | Severity | Helper | Constant |
|---|---|---|---|
| light honk | `light` (info) | `honk.light(title, message)` | `Severity::LIGHT` (= `Severity::Info`) |
| beep-beep | `beep` (success) | `honk.beep(…)` | `Severity::BEEP` |
| loud honk | `loud` (warning) | `honk.loud(…)` | `Severity::LOUD` |
| long honk | `long` (error) | `honk.long(…)` | `Severity::LONG` |
| blast | `blast` (critical) | `honk.blast(…)` | `Severity::BLAST` |

`"loud".parse::<Severity>()`, `"LOUD"` and `"warning"` are the same event (also for
idempotency). `long` and `blast` push at least as `high` priority. `info()`, `success()`,
`warning()`, `error()` and `critical()` remain as synonyms.

Every helper returns a `PendingSend`: chain more fields, then `.await` it.

```rust
use honk_me::Priority;

honk.loud("Disk 91% full", "db-1 /var is at 91%")
    .group_key("disk/db-1/var")
    .priority(Priority::High)
    .metadata("used_percent", 91)
    .await?;
```

## Recipe: notify me when a customer asks for something

Give each request its own group (`requests/<id>`) and its own idempotency key. Two different
customers never fold into one notification, and a retried webhook or job never buzzes twice.

```rust
use honk_me::{Category, Message, Priority, Severity};

let msg = Message::new(format!("{} asked for a quote: {}", req.name, req.summary))
    .title("New quote request")
    .severity(Severity::LOUD)
    .priority(Priority::High)
    .category(Category::Customers)
    .group_key(format!("requests/{}", req.id))
    .url(format!("https://shop.example.com/admin/requests/{}", req.id)); // https only, "Open link"
honk.send(&msg, format!("request-{}", req.id).as_str()).await?;
```

## Problems and recoveries

A problem opens an incident for its group key; a recovery closes it.

```rust
honk.problem("db/backup", "Backup failed", "pg_dump exited with 1").await?;
honk.recovery("db/backup", "Backup OK", "pg_dump finished").await?;
```

## Sending a message

`send(&message, idempotency_key)` returns `Accepted { id, duplicate, received_at }` once Honk
has durably stored the event (`202`); that does not mean a push was delivered.

| Field | Type | Notes |
|---|---|---|
| `message` | `String` | required, 1–8192 bytes of UTF-8 |
| `title` | `Option<String>` | ≤ 160 characters, one line |
| `severity` | `Option<Severity>` | default info |
| `priority` | `Option<Priority>` | `Low`, `Normal`, `High`, `Urgent` (urgent needs a key with `allow_urgent`) |
| `category` | `Option<Category>` | `Infrastructure`, `Security`, `Backups`, `Deployments`, `Payments`, `Customers`, `Sales`, `Automation`, `Personal`, `Other` |
| `source`, `environment`, `channel` | `Option<String>` | ≤ 64 / 32 / 64; fall back to the client's defaults |
| `group_key` | `Option<String>` | ≤ 128; same key, same group |
| `event_type` | `Option<EventType>` | `Event`, `Problem`, `Recovery` (recovery needs a group key) |
| `occurred_at` | `Option<SystemTime>` | sent as RFC 3339 |
| `url`, `image_url` | `Option<String>` | https only, no credentials, ≤ 2048 bytes; no fragment in `image_url` |
| `metadata` | `BTreeMap<String, MetadataValue>` | ≤ 16 keys `[A-Za-z0-9_.-]{1,64}`; strings (≤ 512), numbers, booleans |
| `ttl_seconds` | `Option<u32>` | 60–86400 |
| `source_sequence` | `Option<u64>` | monotonic per source; needs a group key |

`idempotency_key` is your stable key (`"order-42-failed"`, 1–128 printable ASCII characters)
or `None` for a new UUIDv7. A replay within 24 hours returns the original id with
`duplicate: true`; the same key with a different payload is `Error::Conflict`.

## Configuration

```rust
use std::time::Duration;

let honk = Honk::builder()
    .url("https://honk-me.app")
    .key(std::env::var("HONK_KEY").unwrap_or_default())
    .timeout(Duration::from_secs(5))   // per attempt
    .retries(4)                        // after the first attempt; 0 disables
    .deadline(Duration::from_secs(30)) // total, waits included
    .source("billing")                 // defaults for source, environment, channel
    .environment("production")
    .validate(true)                    // false: leave value checks to the server
    .user_agent("my-app/1.2")
    .build()?;
```

Retries happen only on network errors, timeouts, `429` and `5xx`: exponential backoff with full
jitter (`random(0, min(8 s, 0.5 s·2ⁿ))`), never sooner than `Retry-After`, all under the
deadline. A wait that would cross the deadline (a daily quota) fails fast with `retry_after`.
Redirects are reported, never followed.

## Errors

| Variant | When | What to do |
|---|---|---|
| `Error::Validation` | rejected locally (`local == true`) or `400`/`413`/`415`/`422`; `fields` lists every problem | fix the message |
| `Error::Auth` | `401 invalid_key`, `403 priority_not_allowed`, `project_suspended`, `workspace_suspended` | fix the key or the priority |
| `Error::Quota` | `429 quota_exceeded` (daily, resets at UTC midnight), `rate_limited` or `overloaded`, after retries | retry after `retry_after` |
| `Error::Conflict` | `409 idempotency_conflict`: same key, different payload | use a new key, or send the original payload |
| `Error::Network` | unreachable on every attempt | retry later, same key |
| `Error::Timeout` | attempts timed out; the message may or may not be stored | retry later, same key |
| `Error::Server` | `5xx` on every attempt | retry later, same key |
| `Error::Http` | `404` (wrong URL), a redirect, an answer without an id | fix the URL |
| `Error::InvalidConfiguration` | a missing or malformed URL or key | fix the configuration |

Each variant except the last carries a `Failure` with `status`, `code`, `message`, `fields`,
`local`, `request_id`, `idempotency_key`, `attempts` and `retry_after`.

```rust
use honk_me::Error;

match honk.send(&msg, "order-1042-failed").await {
    Ok(_) => {}
    Err(Error::Validation(f)) => tracing::error!(?f.fields, "a bug: don't retry"),
    Err(e) if e.is_retryable() => queue.retry_later(e.idempotency_key(), e.retry_after()),
    Err(e) => return Err(e.into()),
}
```

## Blocking client

```rust
// Cargo.toml: honk-me = { version = "0.1", features = ["blocking"] }
use honk_me::blocking::Honk;

let honk = Honk::from_env()?;
honk.beep("Backup finished", "nightly pg_dump took 42 s").send()?;
honk.send(&honk_me::Message::new("Imported 1 204 rows"), None)?;
```

It runs the async client on a private single-threaded Tokio runtime, with the same retries and
guarantees. Don't call it from inside an async runtime; use the async client there.

## Development

```sh
cargo test                                   # unit tests + mock server
cargo test --all-features                    # also the blocking client
cargo clippy --all-targets --all-features -- -D warnings
HONK_URL=… HONK_KEY=… cargo test --test integration   # against a real server (use a test project's key)
```

The version lives in `Cargo.toml` (`VERSION` and the User-Agent `honk-me-rust/<version>` come
from it). Releases: push a tag `vX.Y.Z` matching it and the release workflow publishes to
crates.io with trusted publishing (see `CHANGELOG.md`).

## Links

- [honk-me.app](https://honk-me.app): the Honk inbox (web, iPhone).
- [docs.rs/honk-me](https://docs.rs/honk-me): the API reference.
- Other SDKs: [Node.js](https://github.com/honk-me/honk-node),
  [PHP / Laravel](https://github.com/honk-me/honk-php), [Go + CLI](https://github.com/honk-me/honk-go),
  [Swift](https://github.com/honk-me/honk-swift), [Kotlin / Java](https://github.com/honk-me/honk-kotlin),
  [n8n](https://github.com/honk-me/honk-n8n).

MIT License.
