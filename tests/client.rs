mod common;

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use common::{KEY, Mock, Reply, client, closed_url};
use honk_me::{Category, Error, EventType, Honk, Message, Priority, Severity};
use serde_json::json;

fn every_field() -> Message {
    Message::new("db-1 /var is at 91%\nsecond line")
        .title("Disk 91% full")
        .severity(Severity::LOUD)
        .priority(Priority::High)
        .category(Category::Infrastructure)
        .source("db-1")
        .environment("production")
        .channel("ops")
        .group_key("disk/db-1/var")
        .event_type(EventType::Problem)
        .occurred_at(UNIX_EPOCH + Duration::from_millis(1_791_116_405_123))
        .url("https://grafana.example.com/d/disk")
        .image_url("https://grafana.example.com/render/disk.png")
        .action("Runbook", "https://wiki.example.com/runbooks/disk")
        .action("Call on-call", "tel:+15550134")
        .metadata("host", "db-1")
        .metadata("used", 91)
        .metadata("ratio", 0.91)
        .metadata("ok", false)
        .ttl_seconds(600)
        .source_sequence(42)
}

#[tokio::test]
async fn sends_every_field_with_the_openapi_names_and_headers() {
    let mock = Mock::start(vec![Reply::accepted("msg_1")]).await;
    let honk = client(&mock).build().unwrap();
    let accepted = honk.send(&every_field(), None).await.unwrap();

    assert_eq!(accepted.id, "msg_1");
    assert!(!accepted.duplicate);
    assert_eq!(accepted.received_at, "2026-10-04T12:20:05.123Z");
    assert_eq!(
        accepted.received_at_time(),
        Some(UNIX_EPOCH + Duration::from_millis(1_791_116_405_123))
    );

    let reqs = mock.requests();
    assert_eq!(reqs.len(), 1);
    let r = &reqs[0];
    assert_eq!(
        (r.method.as_str(), r.path.as_str()),
        ("POST", "/v1/messages")
    );
    assert_eq!(r.header("authorization"), format!("Bearer {KEY}"));
    assert_eq!(r.header("content-type"), "application/json");
    assert_eq!(r.header("accept"), "application/json");
    assert_eq!(
        r.header("user-agent"),
        format!("honk-me-rust/{}", honk_me::VERSION)
    );
    let key = r.header("idempotency-key");
    assert_eq!((key.len(), &key[14..15]), (36, "7"), "a UUIDv7: {key}");
    assert_eq!(
        r.json(),
        json!({
            "title": "Disk 91% full",
            "message": "db-1 /var is at 91%\nsecond line",
            "severity": "warning",
            "priority": "high",
            "category": "infrastructure",
            "source": "db-1",
            "environment": "production",
            "channel": "ops",
            "group_key": "disk/db-1/var",
            "event_type": "problem",
            "occurred_at": "2026-10-04T12:20:05.123Z",
            "url": "https://grafana.example.com/d/disk",
            "image_url": "https://grafana.example.com/render/disk.png",
            "actions": [
                { "title": "Runbook", "url": "https://wiki.example.com/runbooks/disk" },
                { "title": "Call on-call", "url": "tel:+15550134" }
            ],
            "metadata": { "host": "db-1", "used": 91, "ratio": 0.91, "ok": false },
            "ttl_seconds": 600,
            "source_sequence": 42
        })
    );
}

#[tokio::test]
async fn minimal_message_defaults_and_empty_fields() {
    let mock = Mock::start(vec![]).await;
    let honk = client(&mock)
        .source("billing")
        .environment("staging")
        .user_agent("shop/2.1")
        .build()
        .unwrap();
    honk.send(&Message::new("hello"), None).await.unwrap();
    let mut m = Message::new("hi").channel("payments").title("");
    m.source = Some(String::new()); // empty counts as unset: the default applies
    honk.send(&m, "k-1").await.unwrap();

    let reqs = mock.requests();
    assert_eq!(
        reqs[0].json(),
        json!({ "message": "hello", "source": "billing", "environment": "staging" })
    );
    assert_eq!(
        reqs[1].json(),
        json!({ "message": "hi", "source": "billing", "environment": "staging", "channel": "payments" })
    );
    assert_eq!(reqs[1].header("idempotency-key"), "k-1");
    assert_eq!(
        reqs[0].header("user-agent"),
        format!("honk-me-rust/{} shop/2.1", honk_me::VERSION)
    );
}

#[tokio::test]
async fn url_is_normalised() {
    let mock = Mock::start(vec![]).await;
    for url in [
        format!("{}/", mock.url),
        format!("{}/v1/messages", mock.url),
        format!("  {}//  ", mock.url),
    ] {
        Honk::new(url, KEY)
            .unwrap()
            .send(&Message::new("x"), None)
            .await
            .unwrap();
    }
    assert!(mock.requests().iter().all(|r| r.path == "/v1/messages"));
}

#[tokio::test]
async fn helpers_set_severity_event_type_and_group_key() {
    let mock = Mock::start(vec![]).await;
    let honk = client(&mock).build().unwrap();
    honk.problem("db/backup", "Backup failed", "pg_dump exited with 1")
        .await
        .unwrap();
    honk.recovery("db/backup", "Backup OK", "pg_dump finished")
        .await
        .unwrap();
    honk.problem("db/backup", "", "again")
        .severity(Severity::BLAST)
        .action("Call on-call", "tel:+15550134")
        .idempotency_key("p-2")
        .await
        .unwrap();
    for (send, _) in [
        (honk.light("t", "m").send().await, "info"),
        (honk.beep("t", "m").send().await, "success"),
        (honk.loud("t", "m").send().await, "warning"),
        (honk.long("t", "m").send().await, "error"),
        (honk.blast("t", "m").send().await, "critical"),
        (honk.info("t", "m").send().await, "info"),
        (honk.success("t", "m").send().await, "success"),
        (honk.warning("t", "m").send().await, "warning"),
        (honk.error("t", "m").send().await, "error"),
        (honk.critical("t", "m").send().await, "critical"),
    ] {
        send.unwrap();
    }
    let reqs = mock.requests();
    assert_eq!(
        reqs[0].json(),
        json!({ "title": "Backup failed", "message": "pg_dump exited with 1", "severity": "error", "group_key": "db/backup", "event_type": "problem" })
    );
    assert_eq!(reqs[1].json()["severity"], "success");
    assert_eq!(reqs[1].json()["event_type"], "recovery");
    assert_eq!(
        reqs[2].json(),
        json!({ "message": "again", "severity": "critical", "group_key": "db/backup", "event_type": "problem", "actions": [{ "title": "Call on-call", "url": "tel:+15550134" }] })
    );
    assert_eq!(reqs[2].header("idempotency-key"), "p-2");
    let severities: Vec<String> = reqs[3..]
        .iter()
        .map(|r| r.json()["severity"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        severities,
        [
            "info", "success", "warning", "error", "critical", "info", "success", "warning",
            "error", "critical"
        ]
    );
}

#[tokio::test]
async fn retries_503_reusing_the_key_and_body() {
    let mock = Mock::start(vec![
        Reply::error(503, "unavailable", "busy"),
        Reply::error(503, "unavailable", "busy"),
        Reply::accepted("msg_ok"),
    ])
    .await;
    let honk = client(&mock).build().unwrap();
    assert_eq!(honk.send(&every_field(), None).await.unwrap().id, "msg_ok");
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 3);
    assert!(reqs.iter().all(
        |r| r.header("idempotency-key") == reqs[0].header("idempotency-key")
            && r.body == reqs[0].body
    ));
}

#[tokio::test]
async fn retries_dropped_connections_with_the_same_key() {
    let mock = Mock::start(vec![Reply::Drop, Reply::accepted("msg_ok")]).await;
    let honk = client(&mock).build().unwrap();
    honk.send(&Message::new("x"), "drop-1").await.unwrap();
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2);
    assert!(reqs.iter().all(|r| r.header("idempotency-key") == "drop-1"));
}

#[tokio::test]
async fn retries_a_timed_out_attempt_with_the_same_key() {
    let mock = Mock::start(vec![
        Reply::accepted("slow").after(Duration::from_secs(2)),
        Reply::accepted("msg_ok"),
    ])
    .await;
    let honk = client(&mock)
        .timeout(Duration::from_millis(200))
        .build()
        .unwrap();
    assert_eq!(
        honk.send(&Message::new("x"), None).await.unwrap().id,
        "msg_ok"
    );
    let reqs = mock.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(
        reqs[0].header("idempotency-key"),
        reqs[1].header("idempotency-key")
    );
}

#[tokio::test]
async fn honours_retry_after() {
    let mock = Mock::start(vec![
        Reply::error(429, "rate_limited", "slow down").header("Retry-After", "1"),
        Reply::accepted("msg_ok"),
    ])
    .await;
    let honk = client(&mock).build().unwrap();
    let start = Instant::now();
    honk.send(&Message::new("x"), None).await.unwrap();
    assert!(
        start.elapsed() >= Duration::from_secs(1),
        "waited {:?}",
        start.elapsed()
    );
    assert_eq!(mock.count(), 2);
}

#[tokio::test]
async fn retry_after_beyond_the_deadline_fails_fast() {
    let mock = Mock::start(vec![
        Reply::error(429, "quota_exceeded", "daily quota reached").header("Retry-After", "3600"),
    ])
    .await;
    let honk = client(&mock)
        .deadline(Duration::from_secs(5))
        .build()
        .unwrap();
    let start = Instant::now();
    let err = honk.send(&Message::new("x"), "q-1").await.unwrap_err();
    assert!(start.elapsed() < Duration::from_secs(2));
    let Error::Quota(f) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(
        (f.status, f.code.as_str(), f.attempts),
        (Some(429), "quota_exceeded", 1)
    );
    assert_eq!(err.retry_after(), Some(Duration::from_secs(3600)));
    assert_eq!(err.idempotency_key(), Some("q-1"));
    assert!(err.is_retryable());
}

#[tokio::test]
async fn gives_up_after_the_retries() {
    let mock = Mock::start(vec![Reply::error(500, "internal", "boom"); 5]).await;
    let honk = client(&mock).retries(2).build().unwrap();
    let err = honk.send(&Message::new("x"), None).await.unwrap_err();
    let Error::Server(f) = &err else {
        panic!("{err:?}")
    };
    assert_eq!((f.status, f.attempts), (Some(500), 3));
    assert_eq!(mock.count(), 3);
    assert_eq!(err.to_string(), "honk: 500 internal: boom");
}

#[tokio::test]
async fn zero_retries_means_one_attempt() {
    let mock = Mock::start(vec![Reply::error(503, "unavailable", "busy")]).await;
    let err = client(&mock)
        .retries(0)
        .build()
        .unwrap()
        .send(&Message::new("x"), None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Server(_)));
    assert_eq!(mock.count(), 1);
}

#[tokio::test]
async fn stops_at_the_deadline() {
    let mock = Mock::start(vec![
        Reply::accepted("late").after(Duration::from_secs(5));
        20
    ])
    .await;
    let honk = client(&mock)
        .timeout(Duration::from_millis(150))
        .deadline(Duration::from_millis(400))
        .retries(20)
        .build()
        .unwrap();
    let start = Instant::now();
    let err = honk.send(&Message::new("x"), None).await.unwrap_err();
    assert!(matches!(err, Error::Timeout(_)), "{err:?}");
    assert!(
        start.elapsed() < Duration::from_millis(900),
        "took {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn timeout_error_details() {
    let mock = Mock::start(vec![Reply::accepted("late").after(Duration::from_secs(5))]).await;
    let honk = client(&mock)
        .timeout(Duration::from_millis(100))
        .retries(0)
        .build()
        .unwrap();
    let err = honk.send(&Message::new("x"), "t-1").await.unwrap_err();
    let Error::Timeout(f) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(
        (f.code.as_str(), f.status, f.attempts),
        ("timeout", None, 1)
    );
    assert_eq!(err.idempotency_key(), Some("t-1"));
    assert!(err.is_retryable());
    assert!(std::error::Error::source(&err).is_some());
}

#[tokio::test]
async fn connection_refused_is_a_network_error() {
    let honk = Honk::builder()
        .url(closed_url().await)
        .key(KEY)
        .retries(1)
        .backoff(Duration::from_millis(1), Duration::from_millis(2))
        .build()
        .unwrap();
    let err = honk.send(&Message::new("x"), None).await.unwrap_err();
    let Error::Network(f) = &err else {
        panic!("{err:?}")
    };
    assert_eq!((f.code.as_str(), f.attempts), ("network_error", 2));
    assert!(
        err.to_string().starts_with("honk: could not reach Honk"),
        "{err}"
    );
}

#[tokio::test]
async fn errors_that_are_never_retried() {
    for (status, code, want) in [
        (400, "invalid_json", "validation"),
        (401, "invalid_key", "auth"),
        (403, "priority_not_allowed", "auth"),
        (403, "project_suspended", "auth"),
        (404, "not_found", "http"),
        (409, "idempotency_conflict", "conflict"),
        (413, "payload_too_large", "validation"),
        (415, "unsupported_media_type", "validation"),
        (422, "validation_failed", "validation"),
    ] {
        let mock = Mock::start(vec![Reply::error(status, code, "nope")]).await;
        let err = client(&mock)
            .build()
            .unwrap()
            .send(&Message::new("x"), None)
            .await
            .unwrap_err();
        let kind = match &err {
            Error::Validation(_) => "validation",
            Error::Auth(_) => "auth",
            Error::Conflict(_) => "conflict",
            Error::Http(_) => "http",
            other => panic!("{other:?}"),
        };
        assert_eq!(
            (kind, err.status(), err.code()),
            (want, Some(status), Some(code))
        );
        assert!(!err.is_retryable());
        assert_eq!(mock.count(), 1, "{status} is not retried");
        assert_eq!(
            err.failure().unwrap().request_id.as_deref(),
            Some("req_test")
        );
        if status == 404 {
            assert!(err.to_string().contains("base address"), "{err}");
        }
    }
}

#[tokio::test]
async fn server_validation_exposes_every_field() {
    let body = r#"{"error":{"code":"validation_failed","message":"Invalid message","request_id":"req_1","fields":[{"field":"severity","code":"invalid_enum","message":"must be one of info, success"},{"field":"group_key","code":"requires_group_key"}]}}"#;
    let mock = Mock::start(vec![Reply::json(422, body)]).await;
    let err = client(&mock)
        .build()
        .unwrap()
        .send(&Message::new("x"), None)
        .await
        .unwrap_err();
    let Error::Validation(f) = &err else {
        panic!("{err:?}")
    };
    assert!(!f.local);
    assert_eq!(f.fields.len(), 2);
    assert_eq!(
        (
            f.fields[1].field.as_str(),
            f.fields[1].code.as_str(),
            f.fields[1].message.as_str()
        ),
        ("group_key", "requires_group_key", "")
    );
    assert_eq!(
        err.to_string(),
        "honk: 422 validation_failed: Invalid message (severity must be one of info, success; group_key requires_group_key)"
    );
}

#[tokio::test]
async fn redirects_are_reported_not_followed() {
    let mock = Mock::start(vec![
        Reply::text(301, "").header("Location", "https://honk.example.com/v1/messages"),
    ])
    .await;
    let err = client(&mock)
        .build()
        .unwrap()
        .send(&Message::new("x"), None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Http(_)), "{err:?}");
    assert_eq!(err.status(), Some(301));
    assert!(
        err.to_string()
            .contains("redirect to https://honk.example.com/v1/messages"),
        "{err}"
    );
    assert_eq!(mock.count(), 1);
}

#[tokio::test]
async fn a_proxy_html_502_is_retried_then_reported() {
    let mock = Mock::start(vec![Reply::text(502, "<html>Bad gateway</html>"); 2]).await;
    let err = client(&mock)
        .retries(1)
        .build()
        .unwrap()
        .send(&Message::new("x"), None)
        .await
        .unwrap_err();
    let Error::Server(f) = &err else {
        panic!("{err:?}")
    };
    assert_eq!(
        (f.status, f.attempts, f.message.as_str()),
        (Some(502), 2, "<html>Bad gateway</html>")
    );
    assert_eq!(mock.count(), 2);
}

#[tokio::test]
async fn duplicate_and_answers_without_an_id() {
    let mock = Mock::start(vec![
        Reply::duplicate("msg_orig"),
        Reply::json(202, r#"{"status":"accepted"}"#),
    ])
    .await;
    let honk = client(&mock).build().unwrap();
    let a = honk.send(&Message::new("x"), "same").await.unwrap();
    assert!(a.duplicate);
    assert_eq!(a.id, "msg_orig");
    let err = honk.send(&Message::new("x"), None).await.unwrap_err();
    assert!(
        matches!(err, Error::Http(_)) && err.to_string().contains("without a message id"),
        "{err}"
    );
}

#[tokio::test]
async fn validate_false_leaves_value_checks_to_the_server() {
    let mock = Mock::start(vec![]).await;
    let honk = client(&mock).validate(false).build().unwrap();
    honk.send(
        &Message::new("x").title("t".repeat(300)).url("ftp://nope"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        mock.requests()[0].json()["title"].as_str().unwrap().len(),
        300
    );
    let err = honk.send(&Message::new(""), None).await.unwrap_err();
    assert!(
        matches!(&err, Error::Validation(f) if f.local && f.fields[0].code == "required"),
        "{err:?}"
    );
    assert_eq!(mock.count(), 1);
}

#[tokio::test]
async fn invalid_idempotency_keys_are_rejected_locally() {
    let mock = Mock::start(vec![]).await;
    let honk = client(&mock).build().unwrap();
    let long = "k".repeat(129);
    for key in ["", "has space", long.as_str(), "café", "tab\tkey"] {
        let err = honk.send(&Message::new("x"), key).await.unwrap_err();
        assert!(
            matches!(&err, Error::Validation(f) if f.local && f.fields[0].field == "Idempotency-Key"),
            "{key:?}: {err:?}"
        );
    }
    assert_eq!(mock.count(), 0);
    honk.send(&Message::new("x"), "k".repeat(128).as_str())
        .await
        .unwrap();
}

#[test]
fn construction_checks_the_url_and_key() {
    for (url, key, want) in [
        ("", KEY, "URL is required"),
        ("   ", KEY, "URL is required"),
        ("ftp://honk.example.com", KEY, "https://"),
        ("honk.example.com", KEY, "https://"),
        ("https://honk.example.com", "", "key is required"),
        ("https://honk.example.com", "abc", "starting with honk_"),
        (
            "https://honk.example.com",
            "honk_ab cd",
            "starting with honk_",
        ),
    ] {
        let err = Honk::new(url, key).unwrap_err();
        assert!(matches!(err, Error::InvalidConfiguration(_)));
        assert!(err.to_string().contains(want), "{url:?} {key:?}: {err}");
    }
    let honk = Honk::new("https://honk.example.com", format!("  {KEY}\n")).unwrap();
    assert!(
        !format!("{honk:?}").contains(KEY),
        "Debug never prints the key"
    );
}

#[tokio::test]
async fn occurred_at_before_1970_and_now_are_formatted() {
    let mock = Mock::start(vec![]).await;
    let honk = client(&mock).build().unwrap();
    honk.send(
        &Message::new("x").occurred_at(UNIX_EPOCH - Duration::from_millis(1)),
        None,
    )
    .await
    .unwrap();
    honk.send(&Message::new("x").occurred_at(SystemTime::now()), None)
        .await
        .unwrap();
    let reqs = mock.requests();
    assert_eq!(reqs[0].json()["occurred_at"], "1969-12-31T23:59:59.999Z");
    let now = reqs[1].json()["occurred_at"].as_str().unwrap().to_owned();
    assert!(
        now.starts_with("20") && now.ends_with('Z') && now.len() == 24,
        "{now}"
    );
}
