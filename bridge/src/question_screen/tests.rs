use super::*;
use crate::question_match::{pending_claude_spool_questions, pending_codex_questions};
use serde_json::Value;

fn claude_questions(transcript: &[Value]) -> Vec<PendingQuestion> {
    let mut records: Vec<Value> = [
        include_str!("../../../compatibility/phase2/fixtures/claude-question-spool.jsonl"),
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-spool.jsonl"),
    ]
    .into_iter()
    .flat_map(str::lines)
    .map(|l| serde_json::from_str(l).unwrap())
    .collect();
    records.retain(|r| {
        transcript
            .first()
            .is_some_and(|t| r["session_id"] == t["sessionId"])
    });
    pending_claude_spool_questions(&records, transcript)
}

fn pending(kind: QuestionKind) -> PendingQuestion {
    let source = match kind {
        QuestionKind::ClaudeAsk => include_str!(
            "../../../compatibility/phase2/fixtures/claude-question-transcript-pending.jsonl"
        ),
        QuestionKind::CodexPlan => {
            include_str!("../../../compatibility/phase2/fixtures/codex-sync-rollout-pending.jsonl")
        }
        QuestionKind::CodexAsync => {
            include_str!("../../../compatibility/phase2/fixtures/codex-async-rollout-pending.jsonl")
        }
    };
    let records: Vec<_> = source
        .lines()
        .map(|r| serde_json::from_str(r).unwrap())
        .collect();
    if kind == QuestionKind::ClaudeAsk {
        claude_questions(&records)
    } else {
        pending_codex_questions(&records)
    }
    .remove(0)
}
fn fixtures() -> Vec<(QuestionKind, &'static str)> {
    vec![
        (
            QuestionKind::ClaudeAsk,
            include_str!(
                "../../../compatibility/phase2/fixtures/claude-question-single-screen.txt"
            ),
        ),
        (
            QuestionKind::ClaudeAsk,
            include_str!("../../../compatibility/phase2/fixtures/claude-question-multi-screen.txt"),
        ),
        (
            QuestionKind::ClaudeAsk,
            include_str!("../../../compatibility/phase2/fixtures/claude-question-other-screen.txt"),
        ),
        (
            QuestionKind::ClaudeAsk,
            include_str!(
                "../../../compatibility/phase2/fixtures/claude-question-review-screen.txt"
            ),
        ),
        (
            QuestionKind::CodexPlan,
            include_str!("../../../compatibility/phase2/fixtures/codex-sync-first-screen.txt"),
        ),
        (
            QuestionKind::CodexPlan,
            include_str!("../../../compatibility/phase2/fixtures/codex-sync-last-screen.txt"),
        ),
        (
            QuestionKind::CodexPlan,
            include_str!("../../../compatibility/phase2/fixtures/codex-sync-notes-screen.txt"),
        ),
        (
            QuestionKind::CodexAsync,
            include_str!("../../../compatibility/phase2/fixtures/codex-async-queued-screen.txt"),
        ),
        (
            QuestionKind::CodexAsync,
            include_str!("../../../compatibility/phase2/fixtures/codex-async-picker-screen.txt"),
        ),
        (
            QuestionKind::CodexAsync,
            include_str!("../../../compatibility/phase2/fixtures/codex-async-other-screen.txt"),
        ),
    ]
}

#[test]
fn every_screen_fixture_parses_with_expected_step() {
    let steps = [
        QuestionStep::ClaudeQuestion(0),
        QuestionStep::ClaudeQuestion(1),
        QuestionStep::ClaudeQuestion(1),
        QuestionStep::ClaudeReview,
        QuestionStep::CodexQuestion { index: 0, total: 2 },
        QuestionStep::CodexQuestion { index: 1, total: 2 },
        QuestionStep::CodexQuestion { index: 1, total: 2 },
        QuestionStep::AsyncQueued,
        QuestionStep::AsyncPicker(0),
        QuestionStep::AsyncPicker(0),
    ];
    for ((kind, screen), step) in fixtures().into_iter().zip(steps) {
        let dialog = parse(kind, screen, &pending(kind)).unwrap();
        assert_eq!(dialog.step, step);
        assert!(!dialog.fingerprint.is_empty());
        for (i, option) in dialog.options.iter().enumerate() {
            assert_eq!(option.number as usize, i + 1);
        }
        assert!(dialog.options.iter().all(|o| o.label != "Chat about this"));
    }
}

#[test]
fn captures_checked_cursor_and_free_text_states() {
    let screens = fixtures();
    let get = |index: usize| {
        let (kind, screen) = screens[index];
        parse(kind, screen, &pending(kind)).unwrap()
    };
    let single = get(0);
    assert_eq!(single.cursor, Some(QuestionCursor::Option(1)));
    assert_eq!(
        single.options[0].description.as_deref(),
        Some("Choose small")
    );
    assert_eq!(
        single.free_text,
        Some(FreeText {
            active: false,
            text: String::new()
        })
    );
    let multi = get(1);
    assert_eq!(
        multi.options.iter().map(|o| o.checked).collect::<Vec<_>>(),
        vec![Some(true), Some(false), Some(true), Some(false)]
    );
    assert_eq!(
        get(2).free_text,
        Some(FreeText {
            active: true,
            text: "Pepper".into()
        })
    );
    assert_eq!(get(2).cursor, Some(QuestionCursor::Option(4)));
    assert_eq!(get(3).options[0].label, "Submit answers");
    assert_eq!(get(4).cursor, Some(QuestionCursor::Option(1)));
    assert_eq!(get(6).cursor, Some(QuestionCursor::Notes));
    assert_eq!(
        get(6).free_text,
        Some(FreeText {
            active: true,
            text: "Cheese, Basil".into()
        })
    );
    assert_eq!(get(7).countdown.as_deref(), Some("9s"));
    assert!(get(7).options.is_empty());
    assert_eq!(get(8).options[2].label, "Other");
    assert_eq!(
        get(9).free_text,
        Some(FreeText {
            active: true,
            text: "Green".into()
        })
    );
    assert_eq!(get(9).cursor, Some(QuestionCursor::Option(3)));
}

#[test]
fn rejects_mismatches_and_ambiguous_scrollback() {
    for (kind, screen) in fixtures() {
        let request = pending(kind);
        assert!(parse(kind, &format!("{screen}\n{screen}"), &request).is_none());
        if kind == QuestionKind::CodexAsync && screen.contains("⇧← to answer") {
            assert!(parse(kind, &screen.replace("1 question", "0 questions"), &request).is_none());
        } else {
            let wrong = screen
                .replace("Pick a size", "Pick another size")
                .replace("Pick toppings", "Pick other toppings")
                .replace("Pick a color", "Pick another color");
            assert!(parse(kind, &wrong, &request).is_none(), "{screen}");
        }
        if !screen.contains("Review your answers") && !screen.contains("⇧← to answer") {
            let wrong = screen
                .replace("Small", "Tiny")
                .replace("Cheese", "Garlic")
                .replace("Red (Choose red)", "Yellow");
            assert!(parse(kind, &wrong, &request).is_none());
        }
    }
}

#[test]
fn every_dialog_row_affects_fingerprint() {
    for (kind, screen) in fixtures() {
        let request = pending(kind);
        let original = parse(kind, screen, &request).unwrap();
        for index in 0..screen.lines().count() {
            let mut rows: Vec<_> = screen.lines().map(str::to_owned).collect();
            rows[index].push(' ');
            let updated = parse(kind, &rows.join("\n"), &request).unwrap();
            assert_ne!(
                original.fingerprint, updated.fingerprint,
                "row {index}: {screen}"
            );
        }
    }
}

#[test]
fn fingerprint_covers_selection_notes_footer_and_review_answers() {
    for (index, from, to) in [
        (0, "❯ 1. Small", "  1. Small"),
        (1, "[ ] Olives", "[✔] Olives"),
        (2, "Pepper", "Chili"),
        (3, "→ Large", "→ Small"),
        (6, "› Cheese, Basil", "› Olives"),
        (7, "9s", "8s"),
    ] {
        let (kind, screen) = fixtures()[index];
        let request = pending(kind);
        let original = parse(kind, screen, &request).unwrap();
        let changed = parse(kind, &screen.replace(from, to), &request).unwrap();
        assert_ne!(original.fingerprint, changed.fingerprint);
    }
}

#[test]
fn narrow_panes_accept_wrapped_titles_descriptions_and_option_rows() {
    let cases = [
        (
            0,
            vec![
                ("Pick a size", "Pick a\n  size"),
                ("Choose small", "Choose\n       small"),
            ],
        ),
        (
            1,
            vec![
                ("Pick toppings", "Pick top\n  pings"),
                ("Add cheese", "Add\n           cheese"),
            ],
        ),
        (
            4,
            vec![
                ("Pick a size", "Pick a\n  size"),
                (
                    "Small              Choose a small size.",
                    "Small              Choose a\n                       small size.",
                ),
                (
                    "None of the above  Optionally, add details in notes (tab)",
                    "None of the\n       above  Optionally, add\n       details in notes (tab)",
                ),
            ],
        ),
        (
            5,
            vec![
                (
                    "Pick toppings (select all that apply; enter multiple choices in the free-text field).",
                    "Pick toppings (select all that apply;\n  enter multiple choices in the free-text field).",
                ),
                (
                    "Cheese             Add cheese.",
                    "Cheese             Add\n                       cheese.",
                ),
            ],
        ),
        (
            8,
            vec![
                ("Color: Pick a color", "Color: Pick\n  a color"),
                ("Red (Choose red)", "Red (Choose\n       red)"),
            ],
        ),
        (
            9,
            vec![
                ("Color: Pick a color", "Color: Pick\n  a color"),
                ("Blue (Choose blue)", "Blue (Choose\n       blue)"),
            ],
        ),
    ];
    for (index, replacements) in cases {
        let (kind, screen) = fixtures()[index];
        let request = pending(kind);
        let original = parse(kind, screen, &request).unwrap();
        let mut wrapped = screen.to_string();
        for (from, to) in replacements {
            assert!(wrapped.contains(from));
            wrapped = wrapped.replace(from, to);
        }
        let parsed = parse(kind, &wrapped, &request).unwrap_or_else(|| panic!("{wrapped}"));
        assert_ne!(parsed.fingerprint, original.fingerprint);
        assert_eq!(
            parsed,
            QuestionScreen {
                fingerprint: parsed.fingerprint.clone(),
                ..original
            }
        );
        assert!(parse(kind, &format!("{wrapped}\n{wrapped}"), &request).is_none());
        let wrong = wrapped
            .replace("small", "tiny")
            .replace("cheese", "garlic")
            .replace("red)", "yellow)")
            .replace("blue)", "yellow)");
        assert!(parse(kind, &wrong, &request).is_none());
    }
}

#[test]
fn empty_notes_are_active_with_or_without_trailing_space() {
    let (kind, screen) = fixtures()[6];
    for marker in ["› ", "›"] {
        let updated = screen.replace("› Cheese, Basil", marker);
        let parsed = parse(kind, &updated, &pending(kind)).unwrap();
        assert_eq!(parsed.cursor, Some(QuestionCursor::Notes));
        assert_eq!(
            parsed.free_text,
            Some(FreeText {
                active: true,
                text: String::new()
            })
        );
    }
}

#[test]
fn queued_banner_counts_questions_across_calls() {
    let kind = QuestionKind::CodexAsync;
    let first = pending(kind);
    let mut second = first.clone();
    second.request_id = "another-call".into();
    second.questions[0].text = "Another question".into();
    let screen = "• Queued follow-up inputs\n  ? 2 questions · 5s\n    ⇧← to answer\n";
    for request in [first, second] {
        let parsed = parse(kind, screen, &request).unwrap();
        assert_eq!(parsed.queued_count, Some(2));
        assert_eq!(parsed.countdown.as_deref(), Some("5s"));
        assert_eq!(parsed.step, QuestionStep::AsyncQueued);
        for banner in [
            "? 0 questions · 5s",
            "? two questions · 5s",
            "? 2 questions · ",
            "? 2 · 5s",
            "? -1 questions · 5s",
            "? +2 questions · 5s",
        ] {
            assert!(
                parse(
                    kind,
                    &screen.replace("? 2 questions · 5s", banner),
                    &request
                )
                .is_none()
            );
        }
    }
}

#[test]
fn claude_review_accepts_wrapped_questions_and_long_typed_answers() {
    let (kind, screen) = fixtures()[3];
    let request = pending(kind);
    let wrapped = screen
        .replace("● Pick a size", "● Pick a\n   size")
        .replace("● Pick toppings", "● Pick top\n   pings")
        .replace("→ Cheese, Basil, Pepper", "→ Cheese, Basil,\n     Pepper");
    let original = parse(kind, screen, &request).unwrap();
    let parsed = parse(kind, &wrapped, &request).unwrap();
    assert_eq!(parsed.step, QuestionStep::ClaudeReview);
    assert_ne!(parsed.fingerprint, original.fingerprint);
    let long = wrapped.replace(
        "→ Large",
        "→ A long typed answer\n     that continues across\n     several terminal rows",
    );
    assert!(parse(kind, &long, &request).is_some());
    assert!(parse(kind, &long.replace("Pick top", "Choose top"), &request).is_none());
    assert!(parse(kind, &screen.replace("→ Large", "→ \n   "), &request).is_none());
    assert!(parse(kind, &format!("{long}\n{long}"), &request).is_none());
}

#[test]
fn claude_one_question_capture_has_no_review_step() {
    let records = |source: &str| {
        source
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect::<Vec<_>>()
    };
    let requests = claude_questions(&records(include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-one-transcript-pending.jsonl"
    )));
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.questions.len(), 1);
    assert_eq!(request.questions[0].text, "Pick a color?");
    assert!(
        claude_questions(&records(include_str!(
            "../../../compatibility/phase2/fixtures/claude-question-one-transcript.jsonl"
        )))
        .is_empty()
    );
    let screen =
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-screen.txt");
    let kind = QuestionKind::ClaudeAsk;
    for header in ["☐ Color", "☒ Color"] {
        let updated = screen.replace("☐ Color", header);
        let parsed = parse(kind, &updated, request).unwrap();
        assert_eq!(parsed.step, QuestionStep::ClaudeQuestion(0));
        assert_eq!(parsed.options[0].label, "Red");
        assert_eq!(parsed.cursor, Some(QuestionCursor::Option(1)));
        assert!(parse(kind, &updated, &pending(kind)).is_none());
    }
    assert!(
        parse(
            kind,
            &screen.replace("☐ Color", "←  ☐ Color  ✔ Submit  →"),
            request
        )
        .is_none()
    );
    assert!(
        parse(
            kind,
            &screen.replace("Pick a color?", "Pick a color"),
            request
        )
        .is_none()
    );
    assert!(parse(kind, &format!("{screen}\n{screen}"), request).is_none());
    let parsed = parse(kind, screen, request).unwrap();
    for (index, _) in screen.lines().enumerate() {
        let mut rows: Vec<_> = screen.lines().map(str::to_owned).collect();
        rows[index].push(' ');
        assert_ne!(
            parse(kind, &rows.join("\n"), request).unwrap().fingerprint,
            parsed.fingerprint
        );
    }
}

#[test]
fn claude_free_text_continuations_are_joined_for_single_and_multi_select() {
    for (index, from, to, number) in [
        (
            0,
            "  3. Type something.",
            "❯ 3. A long typed\n       answer that spans\n       three rows",
            3,
        ),
        (
            2,
            "❯ 4. [✔] Pepper",
            "❯ 4. [✔] Pepper and\n         more toppings\n         on another row",
            4,
        ),
    ] {
        let (kind, screen) = fixtures()[index];
        let request = pending(kind);
        let original = parse(kind, screen, &request).unwrap();
        let wrapped = screen.replace("❯ 1. Small", "  1. Small").replace(from, to);
        let parsed = parse(kind, &wrapped, &request).unwrap();
        assert_eq!(parsed.cursor, Some(QuestionCursor::Option(number)));
        assert_eq!(
            parsed.free_text,
            Some(FreeText {
                active: true,
                text: if index == 0 {
                    "A long typed answer that spans three rows".into()
                } else {
                    "Pepper and more toppings on another row".into()
                }
            })
        );
        assert_ne!(parsed.fingerprint, original.fingerprint);
        assert!(parse(kind, &format!("{wrapped}\n{wrapped}"), &request).is_none());
        assert!(!parsed.options.iter().any(|o| o.label == "Chat about this"));
        if index == 2 {
            assert_eq!(parsed.options[3].checked, Some(true));
        }
    }
}

#[test]
fn codex_notes_continuations_are_joined_up_to_footer() {
    let (kind, screen) = fixtures()[6];
    let request = pending(kind);
    for (notes, expected) in [
        ("› Cheese,\n     Basil", "Cheese, Basil"),
        (
            "› Cheese,\n     Basil and\n     extra toppings",
            "Cheese, Basil and extra toppings",
        ),
        (
            "›\n     Continued after empty marker\n     on another row",
            "Continued after empty marker on another row",
        ),
        ("›", ""),
        ("› ", ""),
    ] {
        let wrapped = screen.replace("› Cheese, Basil", notes);
        let parsed = parse(kind, &wrapped, &request).unwrap();
        assert_eq!(parsed.cursor, Some(QuestionCursor::Notes));
        assert_eq!(
            parsed.free_text,
            Some(FreeText {
                active: true,
                text: expected.into()
            })
        );
        assert!(!parsed.free_text.unwrap().text.contains("enter to submit"));
    }
}

#[test]
fn every_fixture_survives_hard_wrapping_at_narrow_widths() {
    let mut cases = fixtures()
        .into_iter()
        .map(|(kind, screen)| (pending(kind), screen))
        .collect::<Vec<_>>();
    let records = include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-one-transcript-pending.jsonl"
    )
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect::<Vec<_>>();
    cases.push((
        claude_questions(&records).remove(0),
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-screen.txt"),
    ));
    let mut one_multi = pending(QuestionKind::ClaudeAsk);
    one_multi.questions.remove(0);
    for screen in [
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-multi-screen.txt"),
        include_str!(
            "../../../compatibility/phase2/fixtures/claude-question-one-multi-review-screen.txt"
        ),
    ] {
        cases.push((one_multi.clone(), screen));
    }
    cases.push(first_notes_capture());
    cases.push((
        pending(QuestionKind::CodexAsync),
        include_str!("../../../compatibility/phase2/fixtures/codex-async-queued-nocountdown-screen.txt"),
    ));
    for (request, screen) in cases {
        let original = parse(request.kind, screen, &request).unwrap();
        for width in [40, 30, 24] {
            let wrapped = screen
                .lines()
                .flat_map(|line| {
                    let chars = line.chars().collect::<Vec<_>>();
                    if chars.is_empty() {
                        vec![String::new()]
                    } else {
                        chars
                            .chunks(width)
                            .enumerate()
                            .map(|(index, chunk)| {
                                let text = chunk.iter().collect::<String>();
                                if index > 0 && line.trim().starts_with(['●', '→']) {
                                    format!("     {text}")
                                } else {
                                    text
                                }
                            })
                            .collect()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            let parsed = parse(request.kind, &wrapped, &request)
                .unwrap_or_else(|| panic!("width {width}: {wrapped}"));
            assert_eq!(
                parsed,
                QuestionScreen {
                    fingerprint: parsed.fingerprint.clone(),
                    ..original.clone()
                },
                "width {width}: {wrapped}"
            );
            if screen.lines().any(|line| line.chars().count() > width) {
                assert_ne!(parsed.fingerprint, original.fingerprint);
            }
            assert!(parse(request.kind, &format!("{wrapped}\n{wrapped}"), &request).is_none());
        }
    }
}

#[test]
fn short_control_anchors_also_accept_explicit_continuations() {
    for (index, from, to) in [
        (3, "Review your answers", "Review your\nanswers"),
        (
            3,
            "Ready to submit your answers?",
            "Ready to submit\nyour answers?",
        ),
        (0, "Chat about this", "Chat about\nthis"),
        (7, "? 1 question · 9s", "? 1 ques\ntion · 9s"),
        (7, "⇧← to answer", "⇧← to\nanswer"),
        (8, "• Queued follow-up inputs", "• Queued follow-up\ninputs"),
        (
            8,
            "enter submit   ⌃] skip   ⇧→ main prompt",
            "enter submit\n⌃] skip\n⇧→ main prompt",
        ),
    ] {
        let (kind, screen) = fixtures()[index];
        let request = pending(kind);
        let original = parse(kind, screen, &request).unwrap();
        let wrapped = screen.replace(from, to);
        let parsed = parse(kind, &wrapped, &request).unwrap_or_else(|| panic!("{wrapped}"));
        assert_eq!(
            parsed,
            QuestionScreen {
                fingerprint: parsed.fingerprint.clone(),
                ..original
            }
        );
    }
    let records = include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-one-transcript-pending.jsonl"
    )
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect::<Vec<_>>();
    let request = claude_questions(&records).remove(0);
    let screen =
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-screen.txt");
    assert!(
        parse(
            request.kind,
            &screen.replace("☐ Color", "☐ Col\nor"),
            &request
        )
        .is_some()
    );
}

#[test]
fn footer_lookalikes_in_content_do_not_end_dialogs() {
    for (index, from, to) in [
        (0, "Choose small", "Choose small; Esc to cancel"),
        (4, "Pick a size", "Pick a size; enter to submit all"),
        (
            8,
            "Color: Pick a color",
            "enter submit   ⌃] skip   ⇧→ main prompt",
        ),
    ] {
        let (kind, screen) = fixtures()[index];
        let mut request = pending(kind);
        if index == 0 {
            request.questions[0].options[0].description = Some(to.into());
        } else {
            request.questions[0].text = to.into();
        }
        let updated = screen.replace(from, to);
        let parsed = parse(kind, &updated, &request).unwrap();
        assert_eq!(parsed.options.len(), 3);
        assert!(parse(kind, &format!("{updated}\n{updated}"), &request).is_none());
    }
}

#[test]
fn identical_question_text_requires_exactly_one_full_option_match() {
    for index in [0, 8] {
        let (kind, screen) = fixtures()[index];
        let mut request = pending(kind);
        if kind == QuestionKind::CodexAsync {
            request.questions.push(request.questions[0].clone());
        } else {
            let header = request.questions[1].header.clone();
            request.questions[1] = request.questions[0].clone();
            request.questions[1].header = header;
        }
        request.questions[1].options[0].label = "Different option".into();
        let parsed = parse(kind, screen, &request).unwrap();
        assert_eq!(
            parsed.step,
            if kind == QuestionKind::ClaudeAsk {
                QuestionStep::ClaudeQuestion(0)
            } else {
                QuestionStep::AsyncPicker(0)
            }
        );
        request.questions[1].options[0].label = request.questions[0].options[0].label.clone();
        assert!(parse(kind, screen, &request).is_none());
        request.questions[0].options[0].label = "Wrong first candidate".into();
        let parsed = parse(kind, screen, &request).unwrap();
        assert_eq!(
            parsed.step,
            if kind == QuestionKind::ClaudeAsk {
                QuestionStep::ClaudeQuestion(1)
            } else {
                QuestionStep::AsyncPicker(1)
            }
        );
        request.questions[1].options[0].label = "Wrong second candidate".into();
        assert!(parse(kind, screen, &request).is_none());
    }
}

#[test]
fn structured_content_can_look_like_terminal_controls() {
    for (index, from, description) in [
        (0, "Choose small", "2. Restart the service"),
        (0, "Choose small", "────────────────────────"),
        (4, "Choose a small size.", "› note"),
    ] {
        let (kind, screen) = fixtures()[index];
        let mut request = pending(kind);
        request.questions[0].options[0].description = Some(description.into());
        let updated = screen.replace(from, description);
        let parsed = parse(kind, &updated, &request).unwrap();
        assert_eq!(parsed.options[0].description.as_deref(), Some(description));
    }
    // A control-looking title continuation is also consumed by structured text.
    let (kind, screen) = fixtures()[0];
    let mut request = pending(kind);
    request.questions[0].text = "Pick a size 2. Restart the service".into();
    assert!(
        parse(
            kind,
            &screen.replace("Pick a size", "Pick a size\n  2. Restart the service"),
            &request
        )
        .is_some()
    );
}

#[test]
fn typed_control_lookalikes_use_continuation_indentation() {
    for (index, from, to, expected) in [
        (
            2,
            "❯ 4. [✔] Pepper",
            "❯ 4. [✔] Pepper\n         Submit later",
            "Pepper Submit later",
        ),
        (
            9,
            "› 3. Green",
            "› 3. Green\n       3. another thought",
            "Green 3. another thought",
        ),
    ] {
        let (kind, screen) = fixtures()[index];
        let request = pending(kind);
        let updated = screen.replace(from, to);
        assert_eq!(
            parse(kind, &updated, &request)
                .unwrap()
                .free_text
                .unwrap()
                .text,
            expected
        );
        let ambiguous = updated
            .replace("         Submit later", "  Submit later")
            .replace("       3. another thought", "    3. another thought");
        assert!(parse(kind, &ambiguous, &request).is_none());
    }
}

#[test]
fn review_answer_continuations_keep_bullet_and_arrow_text() {
    let (kind, screen) = fixtures()[3];
    let request = pending(kind);
    for continuation in ["● bullet", "→ arrow", "● bullet\n     → arrow"] {
        let updated = screen.replace("→ Large", &format!("→ Large\n     {continuation}"));
        let parsed = parse(kind, &updated, &request).unwrap();
        assert_eq!(parsed.step, QuestionStep::ClaudeReview);
        assert_ne!(
            parsed.fingerprint,
            parse(kind, screen, &request).unwrap().fingerprint
        );
    }
}

#[test]
fn async_footer_as_typed_continuation_is_not_a_control() {
    let (kind, screen) = fixtures()[9];
    let request = pending(kind);
    let footer = "enter submit   ⌃] skip   ⇧→ main prompt";
    let updated = screen.replace("› 3. Green", &format!("› 3. Green\n       {footer}"));
    let parsed = parse(kind, &updated, &request).unwrap();
    assert_eq!(parsed.free_text.unwrap().text, format!("Green {footer}"));
    let rows = updated.lines().collect::<Vec<_>>();
    assert_eq!(parsed.fingerprint, fingerprint(&rows));
    let truncated = updated.replace(&format!("\n  {footer}"), "");
    assert!(parse(kind, &truncated, &request).is_none());
    // Even a marker at the footer column cannot hide further picker content.
    let followed = updated.replace(
        &format!("\n  {footer}"),
        &format!("\n  {footer}\n       more typed text"),
    );
    assert!(parse(kind, &followed, &request).is_none());
}

#[test]
fn review_title_can_be_an_ordinary_question() {
    let (kind, screen) = fixtures()[0];
    let mut request = pending(kind);
    request.questions[0].text = "Review your answers".into();
    let updated = screen.replace("Pick a size", "Review your answers");
    assert_eq!(
        parse(kind, &updated, &request).unwrap().step,
        QuestionStep::ClaudeQuestion(0)
    );
    let records = include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-one-transcript-pending.jsonl"
    )
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect::<Vec<_>>();
    let mut request = claude_questions(&records).remove(0);
    request.questions[0].text = "Review your answers".into();
    let screen =
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-screen.txt");
    assert_eq!(
        parse(
            kind,
            &screen.replace("Pick a color?", "Review your answers"),
            &request
        )
        .unwrap()
        .step,
        QuestionStep::ClaudeQuestion(0)
    );
}

#[test]
fn arbitrary_control_text_and_truncations_never_panic() {
    let mut screens = fixtures()
        .into_iter()
        .map(|(kind, screen)| (pending(kind), screen))
        .collect::<Vec<_>>();
    let records = include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-one-transcript-pending.jsonl"
    )
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect::<Vec<_>>();
    screens.push((
        claude_questions(&records).remove(0),
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-screen.txt"),
    ));
    let texts = [
        "Enter to select · Esc to cancel",
        "Esc to cancel",
        "enter to submit answer",
        "enter to submit all",
        "enter submit   ⌃] skip   ⇧→ main prompt",
        "• Queued follow-up inputs",
        "? 1 question · 9s",
        "⇧← to answer",
        "Review your answers",
        "Ready to submit your answers?",
        "Submit",
        "Chat about this",
        "Question 1/1 (1 unanswered)",
        "────────────────────────",
        "",
        "● question",
        "→ answer",
        "☐ Color",
    ];
    for (matching, screen) in screens {
        let rows = screen.lines().collect::<Vec<_>>();
        for kind in [
            QuestionKind::ClaudeAsk,
            QuestionKind::CodexPlan,
            QuestionKind::CodexAsync,
        ] {
            let request = if kind == matching.kind {
                matching.clone()
            } else {
                pending(kind)
            };
            for count in 0..=rows.len() {
                let _ = parse(kind, &rows[..count].join("\n"), &request);
                let _ = parse(kind, &rows[count..].join("\n"), &request);
            }
            // Mutating every row is stronger than selecting just content rows.
            for (index, row) in rows.iter().enumerate() {
                for text in texts {
                    for column in [row.chars().take_while(|c| c.is_whitespace()).count(), 2] {
                        let mut changed = rows.iter().map(|r| r.to_string()).collect::<Vec<_>>();
                        changed[index] = format!("{}{text}", " ".repeat(column));
                        let changed = changed.join("\n");
                        let _ = parse(kind, &changed, &request);
                        let mut titled = request.clone();
                        for question in &mut titled.questions {
                            question.text = text.into();
                        }
                        let _ = parse(kind, &changed, &titled);
                    }
                }
            }
        }
    }
}

#[test]
fn markers_inside_typed_text_never_truncate_a_successful_parse() {
    let markers = [
        "Esc to cancel",
        "Enter to select · Esc to cancel",
        "2. Cancel",
        "❯ 2. Cancel",
        "enter to submit answer",
        "enter to submit all",
        "tab or esc to clear notes | enter to submit all",
        "enter submit   ⌃] skip   ⇧→ main prompt",
        "⇧← to answer",
        "• Queued follow-up inputs",
        "? 1 question · 9s",
        "Review your answers",
        "Ready to submit your answers?",
        "Submit",
        "Chat about this",
        "Question 1/1 (1 unanswered)",
        "────────────────────────",
        "←  ☐ Size  ☐ Toppings  ✔ Submit  →",
    ];
    let mut cases = [
        (0, "  3. Type something.", "❯ 3. ", 5),
        (1, "  4. [ ] Type something", "❯ 4. [✔] ", 9),
        (2, "❯ 4. [✔] Pepper", "❯ 4. [✔] ", 9),
        (6, "  › Cheese, Basil", "  › ", 5),
        (8, "    3. Other", "  › 3. ", 7),
        (9, "  › 3. Green", "  › 3. ", 7),
    ]
    .into_iter()
    .map(|(index, old, prefix, column)| {
        let (kind, screen) = fixtures()[index];
        (pending(kind), screen, old, prefix, column)
    })
    .collect::<Vec<_>>();
    let records = include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-one-transcript-pending.jsonl"
    )
    .lines()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect::<Vec<_>>();
    cases.push((
        claude_questions(&records).remove(0),
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-screen.txt"),
        "  3. Type something.",
        "❯ 3. ",
        5,
    ));
    for (request, screen, old, prefix, column) in cases {
        let kind = request.kind;
        for marker in markers {
            for text in [
                format!("text {marker} more"),
                format!("text\n{}{marker}", " ".repeat(column)),
                marker.to_string(),
            ] {
                let updated = screen
                    .replace("❯ 1. Small", "  1. Small")
                    .replace("❯ 1. Red", "  1. Red")
                    .replace("❯ 1. [✔] Cheese", "  1. [✔] Cheese")
                    .replace("  › 1. Red", "    1. Red")
                    .replace(old, &format!("{prefix}{text}"));
                if let Some(parsed) = parse(kind, &updated, &request) {
                    let free = parsed.free_text.unwrap();
                    assert!(
                        compact(&free.text).contains(&compact(marker)),
                        "{updated}\nparsed hint: {}",
                        free.text
                    );
                    let rows = updated.lines().collect::<Vec<_>>();
                    assert_eq!(parsed.fingerprint, fingerprint(&rows), "{updated}");
                }
            }
        }
    }
}

#[test]
fn codex_plan_add_notes_placeholder_is_empty_but_similar_typed_notes_are_preserved() {
    let request = pending(QuestionKind::CodexPlan);
    let screen = include_str!(
        "../../../compatibility/phase2/fixtures/codex-sync-notes-placeholder-screen.txt"
    );
    let parsed = parse(request.kind, screen, &request).unwrap();
    assert_eq!(parsed.cursor, Some(QuestionCursor::Notes));
    assert_eq!(parsed.highlighted, Some(1));
    assert_eq!(
        parsed.free_text,
        Some(FreeText {
            active: true,
            text: String::new()
        })
    );
    assert!(!parsed.is_fresh(&request));
    for notes in [
        "Add notes please",
        "Add  notes",
        "Add notes\n     more text",
    ] {
        let parsed = parse(request.kind, &screen.replace("Add notes", notes), &request).unwrap();
        assert!(!parsed.free_text.unwrap().text.is_empty());
    }
}

#[test]
fn one_question_claude_multiselect_capture_uses_tabs_and_review() {
    let mut request = pending(QuestionKind::ClaudeAsk);
    request.questions.remove(0);
    let screen =
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-multi-screen.txt");
    let review = include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-one-multi-review-screen.txt"
    );
    let question = parse(request.kind, screen, &request).unwrap();
    assert_eq!(question.step, QuestionStep::ClaudeQuestion(0));
    assert!(question.is_fresh(&request));
    assert_eq!(
        question
            .options
            .iter()
            .map(|o| o.checked)
            .collect::<Vec<_>>(),
        vec![Some(false); 4]
    );
    let reviewed = parse(request.kind, review, &request).unwrap();
    assert_eq!(reviewed.step, QuestionStep::ClaudeReview);
    assert_eq!(reviewed.review_answers, vec!["Cheese,Basil"]);
    assert!(!reviewed.is_fresh(&request));
    assert!(
        parse(
            request.kind,
            &screen.replace("←  ☐ Toppings  ✔ Submit  →", " ☐ Toppings"),
            &request
        )
        .is_none()
    );
    request.questions[0].multi_select = false;
    assert!(parse(request.kind, screen, &request).is_none());
    assert!(parse(request.kind, review, &request).is_none());
    let bare = screen
        .replace("←  ☐ Toppings  ✔ Submit  →", " ☐ Toppings")
        .replace("[ ] ", "")
        .replace("Type something", "Type something.")
        .replace("     Submit\n", "");
    assert!(parse(request.kind, &bare, &request).is_some());
}

#[test]
fn claude_first_question_requires_the_cursor_on_option_one_to_be_fresh() {
    let mut multi = pending(QuestionKind::ClaudeAsk);
    multi.questions.remove(0);
    let multi_raw =
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-multi-screen.txt");
    let single = pending(QuestionKind::ClaudeAsk);
    let single_raw =
        include_str!("../../../compatibility/phase2/fixtures/claude-question-single-screen.txt");
    for (request, raw, moved, cursor) in [
        (
            multi,
            multi_raw,
            multi_raw
                .replace("❯ 1.", "  1.")
                .replace("     Submit", "❯    Submit"),
            QuestionCursor::Submit,
        ),
        (
            single,
            single_raw,
            single_raw.replace("❯ 1.", "  1.").replace("  2.", "❯ 2."),
            QuestionCursor::Option(2),
        ),
    ] {
        assert!(
            parse(request.kind, raw, &request)
                .unwrap()
                .is_fresh(&request)
        );
        let parsed = parse(request.kind, &moved, &request).unwrap();
        assert_eq!(parsed.cursor, Some(cursor));
        assert!(!parsed.is_fresh(&request));
        let no_cursor = parse(request.kind, &raw.replace("❯ 1.", "  1."), &request).unwrap();
        assert!(!no_cursor.is_fresh(&request));
    }
}

fn first_notes_capture() -> (PendingQuestion, &'static str) {
    let mut request = pending(QuestionKind::CodexPlan);
    request.questions[0].options[0].description = Some("Choose Small.".into());
    request.questions[0].options[1].description = Some("Choose Large.".into());
    (
        request,
        include_str!("../../../compatibility/phase2/fixtures/codex-sync-notes-first-screen.txt"),
    )
}

#[test]
fn non_last_codex_notes_footer_parses_live_capture() {
    let (request, screen) = first_notes_capture();
    let parsed = parse(request.kind, screen, &request).unwrap();
    assert_eq!(
        parsed.step,
        QuestionStep::CodexQuestion { index: 0, total: 2 }
    );
    assert_eq!(parsed.cursor, Some(QuestionCursor::Notes));
    assert_eq!(parsed.highlighted, Some(3));
    assert_eq!(
        parsed.free_text,
        Some(FreeText {
            active: true,
            text: String::new()
        })
    );
    assert_eq!(parsed.unanswered, Some(2));
}

#[test]
fn async_queued_banner_without_countdown_parses() {
    let screen = include_str!(
        "../../../compatibility/phase2/fixtures/codex-async-queued-nocountdown-screen.txt"
    );
    let request = pending(QuestionKind::CodexAsync);
    let parsed = parse(request.kind, screen, &request).unwrap();
    assert_eq!(parsed.step, QuestionStep::AsyncQueued);
    assert_eq!(parsed.countdown, None);
    assert_eq!(parsed.queued_count, Some(1));
    for notice in [
        "? 0 questions",
        "? two questions",
        "? -1 questions",
        "? 1 question · ",
    ] {
        assert!(
            parse(
                request.kind,
                &screen.replace("? 1 question", notice),
                &request
            )
            .is_none()
        );
    }
    let plural = parse(
        request.kind,
        &screen.replace("? 1 question", "? 2 questions"),
        &request,
    )
    .unwrap();
    assert_eq!(plural.queued_count, Some(2));
    assert_eq!(plural.countdown, None);
}
