use v100_unlock::outcome::{Kind, Outcome};
use v100_unlock::protocol::{encode, parse, Line, HEADER, MAX_TEXT};

#[test]
fn every_line_survives_a_round_trip() {
    let lines = [
        Line::Header,
        Line::Step {
            progress: 0.25,
            text: "card 1/4 · 04:00.0 · 877.5 MHz".into(),
        },
        Line::Note("04:00.0 · V100-SXM2-16GB · 877.5 → 1053.0 MHz".into()),
        Line::End(Outcome::new(
            Kind::Done,
            "hbm 877.5 → 1053.0 MHz (+20%) · 4 cards",
        )),
        Line::End(Outcome::new(Kind::RolledBack, "06:00.0: PLL did not lock")),
    ];
    for line in &lines {
        assert_eq!(parse(&encode(line)).as_ref(), Ok(line));
    }
}

#[test]
fn the_header_carries_the_protocol_version() {
    assert_eq!(encode(&Line::Header), HEADER);
    assert!(parse("v100-unlock 2").is_err());
}

#[test]
fn text_from_the_other_side_cannot_drive_the_terminal() {
    let line = parse("note \u{1b}[2Jgone\u{7}\u{9b}31m").unwrap();
    assert_eq!(line, Line::Note("[2Jgone31m".into()));

    let long = parse(&format!("note {}", "x".repeat(1000))).unwrap();
    assert_eq!(long, Line::Note("x".repeat(MAX_TEXT)));
}

#[test]
fn malformed_lines_are_rejected() {
    for bad in [
        "",
        "step",
        "step 1.5 too far",
        "step -0.1 backwards",
        "step nan what",
        "end maybe text",
        "jump 0.5 x",
    ] {
        assert!(parse(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn a_trailing_carriage_return_from_a_pty_is_ignored() {
    assert_eq!(parse("note hello\r"), Ok(Line::Note("hello".into())));
}

#[test]
fn each_outcome_maps_to_the_state_the_hardware_is_in() {
    assert_eq!(Kind::Done.exit_code(), 0);
    assert_eq!(Kind::Refused.exit_code(), 2);
    assert_eq!(Kind::RolledBack.exit_code(), 3);
    assert_eq!(Kind::Unknown.exit_code(), 4);
}

#[test]
fn a_long_summary_keeps_its_instruction() {
    let summary = format!(
        "a card is in an unknown state, power-cycle the host: {}",
        "x".repeat(250)
    );
    let line = Line::End(Outcome::new(Kind::Unknown, summary));
    assert_eq!(parse(&encode(&line)).as_ref(), Ok(&line));
}
