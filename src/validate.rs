use std::collections::BTreeMap;

use reqwest::Url;
use serde::Serialize;

use crate::error::{Error, FieldError, Result};
use crate::message::{Category, Defaults, EventType, Message, MetadataValue, Priority, Severity};

/// The largest JSON body Honk accepts: 16 KiB.
pub const MAX_BODY_BYTES: usize = 16 << 10;
/// The longest message text, in bytes of UTF-8.
pub const MAX_MESSAGE_BYTES: usize = 8192;
/// The longest title, in characters.
pub const MAX_TITLE: usize = 160;
/// The longest source, in characters.
pub const MAX_SOURCE: usize = 64;
/// The longest environment, in characters.
pub const MAX_ENVIRONMENT: usize = 32;
/// The longest channel, in characters.
pub const MAX_CHANNEL: usize = 64;
/// The longest group key, in characters.
pub const MAX_GROUP_KEY: usize = 128;
/// The longest `url` or `image_url`, in bytes.
pub const MAX_URL_BYTES: usize = 2048;
/// The most metadata keys.
pub const MAX_METADATA_KEYS: usize = 16;
/// The longest metadata string value, in characters.
pub const MAX_METADATA_STRING: usize = 512;
/// The shortest push lifetime, in seconds.
pub const MIN_TTL_SECONDS: u32 = 60;
/// The longest push lifetime, in seconds.
pub const MAX_TTL_SECONDS: u32 = 86_400;
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// The exact JSON of `contracts/openapi.yaml` `MessageRequest`.
#[derive(Serialize)]
struct Wire<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<&'a str>,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    severity: Option<Severity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<Priority>,
    #[serde(skip_serializing_if = "Option::is_none")]
    category: Option<Category>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    environment: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_key: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_type: Option<EventType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    occurred_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<&'a str>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    metadata: &'a BTreeMap<String, MetadataValue>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_sequence: Option<u64>,
}

/// An empty string counts as unset (like `omitempty` in the Go SDK).
fn set(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|s| !s.is_empty())
}

fn with_default<'a>(v: &'a Option<String>, d: &'a Option<String>) -> Option<&'a str> {
    set(v).or_else(|| set(d))
}

/// Validates `message` (unless `validate` is false) with `defaults` applied and returns the
/// exact JSON body that [`Honk::send`](crate::Honk::send) posts. Useful for logging, dry runs
/// and tests.
///
/// ```
/// use honk_me::{encode_message, Defaults, Message, Severity};
///
/// let body = encode_message(&Message::new("Backup finished").severity(Severity::BEEP), &Defaults::new().source("nightly"), true)?;
/// assert_eq!(body, r#"{"message":"Backup finished","severity":"success","source":"nightly"}"#);
/// # Ok::<(), honk_me::Error>(())
/// ```
pub fn encode_message(message: &Message, defaults: &Defaults, validate: bool) -> Result<String> {
    let wire = Wire {
        title: set(&message.title),
        message: &message.message,
        severity: message.severity,
        priority: message.priority,
        category: message.category,
        source: with_default(&message.source, &defaults.source),
        environment: with_default(&message.environment, &defaults.environment),
        channel: with_default(&message.channel, &defaults.channel),
        group_key: set(&message.group_key),
        event_type: message.event_type,
        occurred_at: message.occurred_at.map(crate::time::format_rfc3339),
        url: set(&message.url),
        image_url: set(&message.image_url),
        metadata: &message.metadata,
        ttl_seconds: message.ttl_seconds,
        source_sequence: message.source_sequence,
    };
    if validate {
        let errors = check(&wire);
        if !errors.is_empty() {
            return Err(Error::local_validation(errors));
        }
    } else if message.message.is_empty() {
        return Err(Error::local_validation(vec![FieldError::new(
            "message",
            "required",
            "message is required",
        )]));
    }
    let body = serde_json::to_string(&wire).map_err(|e| {
        Error::local_validation(vec![FieldError::new(
            "body",
            "invalid_format",
            e.to_string(),
        )])
    })?;
    if body.len() > MAX_BODY_BYTES {
        return Err(Error::local_validation(vec![FieldError::new(
            "body",
            "too_long",
            format!(
                "the JSON body is {} bytes; Honk accepts at most 16 KiB",
                body.len()
            ),
        )]));
    }
    Ok(body)
}

/// Unicode control characters and the line/paragraph separators, like the server.
fn has_control(s: &str, allow_breaks: bool) -> bool {
    s.chars().any(|c| {
        if allow_breaks && matches!(c, '\n' | '\t' | '\r') {
            return false;
        }
        c.is_control() || c == '\u{2028}' || c == '\u{2029}'
    })
}

fn short_text(errors: &mut Vec<FieldError>, name: &str, value: Option<&str>, max: usize) {
    let Some(v) = value else { return };
    let s = v.trim();
    if s.is_empty() {
        errors.push(FieldError::new(name, "too_short", "must not be empty"));
    } else if s.chars().count() > max {
        errors.push(FieldError::new(
            name,
            "too_long",
            format!("must be at most {max} characters"),
        ));
    } else if has_control(s, false) {
        errors.push(FieldError::new(
            name,
            "invalid_format",
            "must not contain control characters or line breaks",
        ));
    }
}

/// The server's syntactic check: https, a host, no credentials, at most 2048 bytes; image URLs
/// additionally have no fragment.
pub(crate) fn valid_url(raw: &str, image: bool) -> bool {
    let s = raw.trim();
    if s.is_empty() || s.len() > MAX_URL_BYTES || has_control(s, false) || s.contains([' ', '\\']) {
        return false;
    }
    let Ok(u) = Url::parse(s) else { return false };
    if u.scheme() != "https"
        || u.host_str().is_none_or(str::is_empty)
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return false;
    }
    !(image && s.contains('#'))
}

/// 1–128 printable ASCII characters (0x21–0x7E).
pub(crate) fn valid_idempotency_key(k: &str) -> bool {
    (1..=128).contains(&k.len()) && k.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

fn valid_metadata_key(k: &str) -> bool {
    (1..=64).contains(&k.len())
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

fn check(m: &Wire<'_>) -> Vec<FieldError> {
    let mut f = Vec::new();
    let text = m.message;
    if text.is_empty() {
        f.push(FieldError::new(
            "message",
            "required",
            "message is required",
        ));
    } else if text.len() > MAX_MESSAGE_BYTES {
        f.push(FieldError::new(
            "message",
            "too_long",
            format!(
                "must be at most {MAX_MESSAGE_BYTES} bytes of UTF-8 (got {})",
                text.len()
            ),
        ));
    } else if text.trim().is_empty() {
        f.push(FieldError::new("message", "too_short", "must not be blank"));
    } else if has_control(text, true) {
        f.push(FieldError::new(
            "message",
            "invalid_format",
            "must not contain control characters other than line breaks and tabs",
        ));
    }
    short_text(&mut f, "title", m.title, MAX_TITLE);
    short_text(&mut f, "source", m.source, MAX_SOURCE);
    short_text(&mut f, "environment", m.environment, MAX_ENVIRONMENT);
    short_text(&mut f, "channel", m.channel, MAX_CHANNEL);
    short_text(&mut f, "group_key", m.group_key, MAX_GROUP_KEY);

    if m.source_sequence.is_some_and(|n| n > MAX_SAFE_INTEGER) {
        f.push(FieldError::new(
            "source_sequence",
            "out_of_range",
            "must be between 0 and 2^53-1",
        ));
    }
    if m.group_key.is_none() {
        if m.event_type == Some(EventType::Recovery) {
            f.push(FieldError::new(
                "group_key",
                "requires_group_key",
                "recovery events require group_key",
            ));
        }
        if m.source_sequence.is_some() {
            f.push(FieldError::new(
                "source_sequence",
                "requires_group_key",
                "source_sequence requires group_key",
            ));
        }
    }
    if m.url.is_some_and(|u| !valid_url(u, false)) {
        f.push(FieldError::new(
            "url",
            "invalid_format",
            "must be an https URL without credentials, at most 2048 bytes",
        ));
    }
    if m.image_url.is_some_and(|u| !valid_url(u, true)) {
        f.push(FieldError::new(
            "image_url",
            "invalid_format",
            "must be an https URL without credentials or fragment, at most 2048 bytes",
        ));
    }

    if m.metadata.len() > MAX_METADATA_KEYS {
        f.push(FieldError::new(
            "metadata",
            "too_long",
            format!("at most {MAX_METADATA_KEYS} keys"),
        ));
    }
    for (k, v) in m.metadata {
        let field = format!("metadata.{k}");
        if !valid_metadata_key(k) {
            f.push(FieldError::new(
                field,
                "invalid_format",
                "keys must match [A-Za-z0-9_.-]{1,64}",
            ));
            continue;
        }
        let problem = match v {
            MetadataValue::String(s)
                if s.chars().count() > MAX_METADATA_STRING || has_control(s, true) =>
            {
                Some(format!(
                    "strings must be at most {MAX_METADATA_STRING} characters, without control characters"
                ))
            }
            MetadataValue::Float(x) if !x.is_finite() => Some("numbers must be finite".to_owned()),
            _ => None,
        };
        if let Some(msg) = problem {
            f.push(FieldError::new(field, "invalid_format", msg));
        }
    }

    if m.ttl_seconds
        .is_some_and(|t| !(MIN_TTL_SECONDS..=MAX_TTL_SECONDS).contains(&t))
    {
        f.push(FieldError::new(
            "ttl_seconds",
            "out_of_range",
            format!("must be between {MIN_TTL_SECONDS} and {MAX_TTL_SECONDS}"),
        ));
    }
    f
}
