//! Against a real Honk server: runs when HONK_URL and HONK_KEY are set (sdk/scripts/integration.sh
//! starts a throwaway one), skipped otherwise. The key must not allow urgent priority.
use honk_me::{Action, Category, Error, EventType, Honk, Message, Priority, Severity};

fn honk() -> Option<Honk> {
    match (std::env::var("HONK_URL"), std::env::var("HONK_KEY")) {
        (Ok(url), Ok(key)) if !url.is_empty() && !key.is_empty() => {
            Some(Honk::new(url, key).unwrap())
        }
        _ => {
            eprintln!("skipped: HONK_URL / HONK_KEY not set");
            None
        }
    }
}

fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", honk_me::new_idempotency_key())
}

#[tokio::test]
async fn integration_minimal() {
    let Some(honk) = honk() else { return };
    let a = honk
        .send(&Message::new("rust: minimal"), None)
        .await
        .unwrap();
    assert!(a.id.starts_with("msg_"), "{a:?}");
    assert!(!a.duplicate);
    assert!(a.received_at_time().is_some(), "{a:?}");
}

#[tokio::test]
async fn integration_every_field_and_duplicate() {
    let Some(honk) = honk() else { return };
    let key = unique("rust-all");
    let msg = Message::new("rust: every field")
        .title("Rust SDK")
        .severity(Severity::LOUD)
        .priority(Priority::High)
        .category(Category::Automation)
        .source("rust-sdk")
        .environment("test")
        .channel("sdk")
        .group_key(unique("rust/group"))
        .occurred_at(std::time::SystemTime::now())
        .url("https://honk-me.app")
        .image_url("https://honk-me.app/icon.png")
        .metadata("lang", "rust")
        .metadata("n", 1)
        .ttl_seconds(600);
    let first = honk.send(&msg, key.as_str()).await.unwrap();
    let again = honk.send(&msg, key.as_str()).await.unwrap();
    assert!(again.duplicate);
    assert_eq!(first.id, again.id);
}

#[tokio::test]
async fn integration_actions_and_duplicate() {
    let Some(honk) = honk() else { return };
    let key = unique("rust-actions");
    let mut msg = Message::new("rust: Emily Carter asked for a quote")
        .title("New quote request")
        .group_key(unique("rust/requests"))
        .action("Reply", "mailto:emily@example.com?subject=Your%20quote")
        .action("Call", "tel:+15550134");
    let first = honk.send(&msg, key.as_str()).await.unwrap();
    let again = honk.send(&msg, key.as_str()).await.unwrap();
    assert!(again.duplicate);
    assert_eq!(first.id, again.id);
    // The actions are part of the idempotency payload.
    msg.actions = vec![Action::new("Reply", "mailto:emily@example.com")];
    let err = honk.send(&msg, key.as_str()).await.unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(f) if f.code == "idempotency_conflict"),
        "{err:?}"
    );
}

#[tokio::test]
async fn integration_conflict() {
    let Some(honk) = honk() else { return };
    let key = unique("rust-conflict");
    honk.send(&Message::new("rust: one"), key.as_str())
        .await
        .unwrap();
    let err = honk
        .send(&Message::new("rust: two"), key.as_str())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(f) if f.code == "idempotency_conflict"),
        "{err:?}"
    );
}

#[tokio::test]
async fn integration_problem_and_recovery() {
    let Some(honk) = honk() else { return };
    let group = unique("rust/incident");
    honk.problem(&group, "Rust problem", "it broke")
        .await
        .unwrap();
    honk.recovery(&group, "Rust recovery", "it works")
        .await
        .unwrap();
}

#[tokio::test]
async fn integration_wrong_key() {
    let Some(_) = honk() else { return };
    let url = std::env::var("HONK_URL").unwrap();
    let bad = Honk::new(url, "honk_000000000000_00000000000000000000000000000000").unwrap();
    let err = bad
        .send(&Message::new("rust: wrong key"), None)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Auth(f) if f.code == "invalid_key"),
        "{err:?}"
    );
}

#[tokio::test]
async fn integration_urgent_not_allowed() {
    let Some(honk) = honk() else { return };
    let err = honk
        .send(
            &Message::new("rust: urgent").priority(Priority::Urgent),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Auth(f) if f.code == "priority_not_allowed"),
        "{err:?}"
    );
}

#[tokio::test]
async fn integration_server_validation() {
    let Some(_) = honk() else { return };
    let honk = Honk::builder()
        .url(std::env::var("HONK_URL").unwrap())
        .key(std::env::var("HONK_KEY").unwrap())
        .validate(false)
        .build()
        .unwrap();
    let err = honk
        .send(
            &Message::new("rust: invalid").event_type(EventType::Recovery),
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Validation(f) if !f.local && f.fields.iter().any(|e| e.field == "group_key")),
        "{err:?}"
    );
}

#[tokio::test]
async fn integration_server_validation_of_actions() {
    let Some(_) = honk() else { return };
    let honk = Honk::builder()
        .url(std::env::var("HONK_URL").unwrap())
        .key(std::env::var("HONK_KEY").unwrap())
        .validate(false)
        .build()
        .unwrap();
    let msg = Message::new("rust: invalid action")
        .action("Call", "tel:+15550134")
        .action("Open", "javascript:alert(1)");
    let err = honk.send(&msg, None).await.unwrap_err();
    assert!(
        matches!(&err, Error::Validation(f) if !f.local && f.fields.iter().map(|e| (e.field.as_str(), e.code.as_str())).eq([("actions[1].url", "invalid_format")])),
        "{err:?}"
    );
}
