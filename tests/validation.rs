use honk_me::{
    Category, Defaults, Error, EventType, Message, MetadataValue, Priority, Severity,
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
