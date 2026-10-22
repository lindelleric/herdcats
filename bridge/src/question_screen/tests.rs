use super::*;
use crate::question_match::{pending_claude_questions, pending_codex_questions};

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
        pending_claude_questions(&records)
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
            assert!(parse(kind, &screen.replace("1 question", "2 questions"), &request).is_none());
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
