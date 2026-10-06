use honk_me::{
    Action, Category, Defaults, Error, EventType, Message, MetadataValue, Priority, Severity,
    encode_message,
};

fn fields(m: &Message) -> Vec<(String, String)> {
    match encode_message(m, &Defaults::new(), true) {
        Ok(_) => vec![],
        Err(Error::Validation(f)) => {
            assert!(f.local);
            assert_eq!(f.code, "validation_failed");
            f.fields.into_iter().map(|e| (e.field, e.code)).collect()
        }
        Err(other) => panic!("{other:?}"),
    }
}

fn one(m: Message) -> (String, String) {
    let f = fields(&m);
    assert_eq!(f.len(), 1, "{f:?}");
    f.into_iter().next().unwrap()
}

#[test]
fn each_rule() {
    let s = |n: usize| "x".repeat(n);
    let long_key = format!("metadata.{}", s(65));
    let cases: Vec<(Message, &str, &str)> = vec![
        (Message::new(""), "message", "required"),
        (Message::new("x".repeat(8193)), "message", "too_long"),
        (Message::new("   \n "), "message", "too_short"),
        (Message::new("bell\u{7}"), "message", "invalid_format"),
        (Message::new("sep\u{2028}"), "message", "invalid_format"),
        (Message::new("m").title(s(161)), "title", "too_long"),
        (Message::new("m").title("  "), "title", "too_short"),
        (
            Message::new("m").title("two\nlines"),
            "title",
            "invalid_format",
        ),
        (Message::new("m").source(s(65)), "source", "too_long"),
        (
            Message::new("m").environment(s(33)),
            "environment",
            "too_long",
        ),
        (Message::new("m").channel(s(65)), "channel", "too_long"),
        (Message::new("m").group_key(s(129)), "group_key", "too_long"),
        (
            Message::new("m").event_type(EventType::Recovery),
            "group_key",
            "requires_group_key",
        ),
        (
            Message::new("m").group_key("g").source_sequence(1 << 53),
            "source_sequence",
            "out_of_range",
        ),
        (
            Message::new("m").url("http://example.com"),
            "url",
            "invalid_format",
        ),
        (
            Message::new("m").url("https://user:pw@example.com"),
            "url",
            "invalid_format",
        ),
        (
            Message::new("m").url(format!("https://example.com/{}", s(2040))),
            "url",
            "invalid_format",
        ),
        (
            Message::new("m").url("https://exa mple.com"),
            "url",
            "invalid_format",
        ),
        (
            Message::new("m").image_url("https://example.com/a.png#frag"),
            "image_url",
            "invalid_format",
        ),
        (
            Message::new("m").image_url("not a url"),
            "image_url",
            "invalid_format",
        ),
        (
            Message::new("m").metadata("bad key", 1),
            "metadata.bad key",
            "invalid_format",
        ),
        (
            Message::new("m").metadata(s(65), 1),
            long_key.as_str(),
            "invalid_format",
        ),
        (
            Message::new("m").metadata("long", s(513)),
            "metadata.long",
            "invalid_format",
        ),
        (
            Message::new("m").metadata("ctl", "a\u{0}b"),
            "metadata.ctl",
            "invalid_format",
        ),
        (
            Message::new("m").metadata("nan", f64::NAN),
            "metadata.nan",
            "invalid_format",
        ),
        (
            Message::new("m").ttl_seconds(59),
            "ttl_seconds",
            "out_of_range",
        ),
        (
            Message::new("m").ttl_seconds(86_401),
            "ttl_seconds",
            "out_of_range",
        ),
    ];
    for (m, field, code) in cases {
        assert_eq!(
            one(m),
            (field.to_owned(), code.to_owned()),
            "{field} {code}"
        );
    }

    let mut many = Message::new("m");
    for i in 0..17 {
        many = many.metadata(format!("k{i}"), i);
    }
    assert_eq!(one(many), ("metadata".to_owned(), "too_long".to_owned()));

    let seq_without_group = fields(&Message::new("m").source_sequence(3));
    assert_eq!(
        seq_without_group,
        vec![(
            "source_sequence".to_owned(),
            "requires_group_key".to_owned()
        )]
    );
}

#[test]
fn each_action_rule() {
    let action = |title: &str, url: &str| Message::new("m").action(title, url);
    let cases: Vec<(Message, &str, &str)> = vec![
        (action("  ", "tel:1"), "actions[0].title", "required"),
        (
            action(&"t".repeat(41), "tel:1"),
            "actions[0].title",
            "too_long",
        ),
        (
            action("Call\nEmily", "tel:1"),
            "actions[0].title",
            "invalid_format",
        ),
        (
            action("Call\u{7}", "tel:1"),
            "actions[0].title",
            "invalid_format",
        ),
        (action("Open", " "), "actions[0].url", "required"),
        (
            action("Open", &format!("https://example.com/{}", "a".repeat(2029))),
            "actions[0].url",
            "too_long",
        ),
        (
            action("Open", "http://example.com"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Open", "javascript:alert(1)"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Open", "shop://orders/42"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Open", "https://user:pw@example.com"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Open", "https:///orders"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Open", "https://@example.com"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Reply", "mailto:emily@example.com?subject=Your quote"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Reply", "mailto:?subject=Hi"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Reply", "mailto:emily@localhost"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Reply", "mailto:emily@example.com,bob@example.com"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Reply", "mailto:Emily%20%3Cemily@example.com%3E"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Reply", "mailto:emily..carter@example.com"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Reply", "mailto:emily@example.com?cc=boss@example.com"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Reply", "mailto:emily@example.com?subject=100%"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Call", "tel:+1\u{a0}5550134"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Call", "tel:+1-555-CALL"),
            "actions[0].url",
            "invalid_format",
        ),
        (action("Call", "tel:+"), "actions[0].url", "invalid_format"),
        (
            action("Call", "tel:+15550134?x=1"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Text", "sms://+15550134"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Text", "sms:+15550134?subject=Hi"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            action("Text", "sms:+15550134?body=a;b"),
            "actions[0].url",
            "invalid_format",
        ),
        (
            Message::new("m")
                .action("Call", "tel:1")
                .action("Open", "ftp://example.com"),
            "actions[1].url",
            "invalid_format",
        ),
    ];
    for (m, field, code) in cases {
        assert_eq!(
            one(m),
            (field.to_owned(), code.to_owned()),
            "{field} {code}"
        );
    }

    let mut four = Message::new("m");
    for _ in 0..4 {
        four = four.action("Call", "tel:+15550134");
    }
    assert_eq!(one(four), ("actions".to_owned(), "too_long".to_owned()));

    let m = Message::new("")
        .url("ftp://a")
        .action("", "ftp://b")
        .action("Call", "tel:1")
        .ttl_seconds(5);
    assert_eq!(
        fields(&m),
        [
            ("message", "required"),
            ("url", "invalid_format"),
            ("actions[0].title", "required"),
            ("actions[0].url", "invalid_format"),
            ("ttl_seconds", "out_of_range"),
        ]
        .map(|(f, c)| (f.to_owned(), c.to_owned()))
    );
}

#[test]
fn actions_that_pass_and_their_json() {
    for url in [
        "https://shop.example.com/admin/orders/42?tab=notes#latest",
        "HTTPS://example.com:8443/a",
        "mailto:emily@example.com",
        "MailTo:emily@example.com?subject=Your%20quote&body=Hi%20Emily",
        "mailto:first.last+quotes@example.co.uk?subject=Your+quote",
        "mailto:emily%40example.com",
        "mailto:ana@[192.0.2.1]",
        "tel:+15550134",
        "tel:+1-555-013.4",
        "tel:(555)0134",
        "TEL://+15550134",
        "sms:+15550134",
        "SMS:5550134?body=On%20my%20way",
        "sms:+15550134?",
        &format!("https://example.com/{}", "a".repeat(2028)),
    ] {
        let body = encode_message(
            &Message::new("x").action("Open", url),
            &Defaults::new(),
            true,
        )
        .unwrap_or_else(|e| panic!("{url}: {e}"));
        let json: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            json["actions"],
            serde_json::json!([{ "title": "Open", "url": url }])
        );
    }
    let titles = Message::new("x")
        .action("é".repeat(40), "tel:1")
        .action("  Call Emily Carter  ", "tel:1")
        .action("Reply ✉️", "mailto:a@b.co");
    assert_eq!(fields(&titles), vec![]);
    assert_eq!(
        titles.actions[1],
        Action::new("  Call Emily Carter  ", "tel:1")
    );
    // No actions: no "actions" key at all.
    assert_eq!(
        encode_message(&Message::new("x"), &Defaults::new(), true).unwrap(),
        r#"{"message":"x"}"#
    );
}

#[test]
fn every_problem_is_reported_at_once() {
    let m = Message::new("")
        .title("t".repeat(200))
        .url("ftp://x")
        .ttl_seconds(5)
        .event_type(EventType::Recovery)
        .metadata("bad key", true);
    let got: Vec<String> = fields(&m).into_iter().map(|(f, _)| f).collect();
    assert_eq!(
        got,
        [
            "message",
            "title",
            "group_key",
            "url",
            "metadata.bad key",
            "ttl_seconds"
        ]
    );
    let err = encode_message(&m, &Defaults::new(), true).unwrap_err();
    assert!(
        err.to_string().starts_with(
            "honk: invalid message (message message is required; title must be at most 160"
        ),
        "{err}"
    );
}

#[test]
fn edge_cases_that_pass() {
    let ok = [
        Message::new("x".repeat(8192)),
        Message::new("é".repeat(4096)), // 8192 bytes
        Message::new("tabs\tand\r\nbreaks"),
        Message::new("m")
            .title("é".repeat(160))
            .group_key("g".repeat(128))
            .source("s".repeat(64)),
        Message::new("m")
            .environment("e".repeat(32))
            .channel("c".repeat(64)),
        Message::new("m")
            .url("https://example.com:8443/a?b=c#d")
            .image_url("https://example.com/a.png?x=1"),
        Message::new("m")
            .group_key("g")
            .source_sequence((1 << 53) - 1)
            .event_type(EventType::Recovery),
        Message::new("m").ttl_seconds(60),
        Message::new("m").ttl_seconds(86_400),
        Message::new("m")
            .metadata("a.b-c_d", "x".repeat(512))
            .metadata("n", -3)
            .metadata("f", 1.5)
            .metadata("b", true),
    ];
    for m in ok {
        assert_eq!(fields(&m), vec![], "{:?}", m.title);
    }
}

#[test]
fn the_16_kib_body_limit() {
    let mut m = Message::new("x".repeat(8192)).title("t".repeat(160)); // every field within its own limit
    for i in 0..16 {
        m = m.metadata(format!("key{i}"), "v".repeat(500));
    }
    let err = encode_message(&m, &Defaults::new(), true).unwrap_err();
    assert!(
        matches!(&err, Error::Validation(f) if f.fields[0].field == "body" && f.fields[0].code == "too_long"),
        "{err:?}"
    );
}

#[test]
fn encode_applies_defaults_and_is_canonical() {
    let d = Defaults::new()
        .source("nightly")
        .environment("prod")
        .channel("ops");
    let body = encode_message(
        &Message::new("Backup finished")
            .severity(Severity::BEEP)
            .channel("db"),
        &d,
        true,
    )
    .unwrap();
    assert_eq!(
        body,
        r#"{"message":"Backup finished","severity":"success","source":"nightly","environment":"prod","channel":"db"}"#
    );
    // The same event with a horn alias or the canonical value is the same payload (and so the
    // same idempotency payload).
    let a = encode_message(&Message::new("x").severity(Severity::LOUD), &d, true).unwrap();
    let b = encode_message(&Message::new("x").severity(Severity::Warning), &d, true).unwrap();
    assert_eq!(a, b);
    // Metadata keys are sorted, so the body is deterministic.
    let m = encode_message(
        &Message::new("x").metadata("b", 1).metadata("a", 2),
        &Defaults::new(),
        true,
    )
    .unwrap();
    assert_eq!(m, r#"{"message":"x","metadata":{"a":2,"b":1}}"#);
}

#[test]
fn the_honk_scale() {
    assert_eq!(
        [
            Severity::LIGHT,
            Severity::BEEP,
            Severity::LOUD,
            Severity::LONG,
            Severity::BLAST
        ],
        [
            Severity::Info,
            Severity::Success,
            Severity::Warning,
            Severity::Error,
            Severity::Critical
        ]
    );
    for (input, want) in [
        ("light", Severity::Info),
        ("BEEP", Severity::Success),
        (" Loud ", Severity::Warning),
        ("long", Severity::Error),
        ("blast", Severity::Critical),
        ("warning", Severity::Warning),
        ("CRITICAL", Severity::Critical),
    ] {
        assert_eq!(input.parse::<Severity>().unwrap(), want, "{input}");
    }
    let err = "honk".parse::<Severity>().unwrap_err().to_string();
    assert!(
        err.contains("light (info)") && err.contains("blast (critical)"),
        "{err}"
    );
    assert_eq!(Severity::Critical.to_string(), "critical");
    assert_eq!(Severity::Critical.horn(), "blast");
}

#[test]
fn other_enums_parse_and_print_their_wire_values() {
    assert_eq!("HIGH".parse::<Priority>().unwrap(), Priority::High);
    assert_eq!(
        "recovery".parse::<EventType>().unwrap(),
        EventType::Recovery
    );
    assert_eq!("payments".parse::<Category>().unwrap(), Category::Payments);
    assert!(
        "soon"
            .parse::<Priority>()
            .unwrap_err()
            .to_string()
            .contains("low, normal, high, urgent")
    );
    assert_eq!(Category::ALL.len(), 10);
    assert_eq!(Priority::Urgent.to_string(), "urgent");
    assert_eq!(MetadataValue::from(7u8), MetadataValue::Integer(7));
    assert_eq!(MetadataValue::from("x"), MetadataValue::String("x".into()));
}
