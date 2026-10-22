use super::*;
use crate::{question_match, question_screen};
use serde_json::json;
fn fixture(kind: QuestionKind, screen: &str) -> (PendingQuestion, QuestionScreen) {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../compatibility/phase2/fixtures");
    let source = match kind {
        QuestionKind::ClaudeAsk => "claude-question-transcript",
        QuestionKind::CodexPlan => "codex-sync-rollout",
        QuestionKind::CodexAsync => "codex-async-rollout",
    };
    let records: Vec<Value> = std::fs::read_to_string(root.join(format!("{source}-pending.jsonl")))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let pending = if kind == QuestionKind::ClaudeAsk {
        question_match::pending_claude_spool_questions(
            &std::fs::read_to_string(root.join("claude-question-spool.jsonl"))
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect::<Vec<_>>(),
            &records,
        )
    } else {
        question_match::pending_codex_questions(&records)
    }
    .remove(0);
    let screen = question_screen::parse(
        kind,
        &std::fs::read_to_string(root.join(format!("{screen}-screen.txt"))).unwrap(),
        &pending,
    )
    .unwrap();
    (pending, screen)
}
fn dialog(step: QuestionStep, cursor: QuestionCursor) -> ExpectedDialog {
    ExpectedDialog {
        step,
        cursor: Some(cursor),
        checked: vec![],
        text: String::new(),
        text_active: false,
        unanswered: None,
        review_answers: vec![],
        notes_option: None,
    }
}
fn key(key: &str, expected: ExpectedScreen) -> Step {
    Step {
        input: Input::Keys(vec![key.into()]),
        expected,
        only_if: None,
    }
}
fn text(value: &str, expected: ExpectedDialog) -> Step {
    Step {
        input: Input::Text(value.into()),
        expected: ExpectedScreen::Dialog(expected),
        only_if: None,
    }
}
#[test]
fn async_option_from_banner_opens_selects_and_submits() {
    for name in ["codex-async-queued", "codex-async-queued-nocountdown"] {
        let (pending, screen) = fixture(QuestionKind::CodexAsync, name);
        let picker = dialog(QuestionStep::AsyncPicker(0), QuestionCursor::Option(1));
        let selected = dialog(QuestionStep::AsyncPicker(0), QuestionCursor::Option(2));
        assert_eq!(
            plan(
                pending.kind,
                &pending,
                &screen,
                &json!({"answers":[{"index":0,"options":[2]}]})
            )
            .unwrap(),
            vec![
                key("shift+left", ExpectedScreen::Dialog(picker)),
                key("2", ExpectedScreen::Dialog(selected)),
                key("enter", ExpectedScreen::Gone)
            ]
        );
    }
}

#[test]
fn async_other_and_skip_from_open_and_queued() {
    for start in ["codex-async-picker", "codex-async-queued"] {
        let (pending, screen) = fixture(QuestionKind::CodexAsync, start);
        let picker = dialog(QuestionStep::AsyncPicker(0), QuestionCursor::Option(1));
        let mut prefix = vec![];
        if start.ends_with("queued") {
            prefix.push(key("shift+left", ExpectedScreen::Dialog(picker)));
        }
        let mut other = dialog(QuestionStep::AsyncPicker(0), QuestionCursor::Option(3));
        other.text_active = true;
        let mut typed = other.clone();
        typed.text = "Green".into();
        let mut want = prefix.clone();
        want.extend([
            key("3", ExpectedScreen::Dialog(other)),
            text("Green", typed),
            key("enter", ExpectedScreen::Gone),
        ]);
        assert_eq!(
            plan(
                pending.kind,
                &pending,
                &screen,
                &json!({"answers":[{"index":0,"options":[],"text":"Green"}]})
            )
            .unwrap(),
            want
        );
        let mut want = prefix;
        want.push(key("ctrl+]", ExpectedScreen::Gone));
        assert_eq!(
            plan(pending.kind, &pending, &screen, &json!({"skip":true})).unwrap(),
            want
        );
    }
}
#[test]
fn claude_tabs_toggle_type_review_and_submit() {
    let (pending, screen) = fixture(QuestionKind::ClaudeAsk, "claude-question-single");
    let mut multi = dialog(QuestionStep::ClaudeQuestion(1), QuestionCursor::Option(1));
    let mut want = vec![key("2", ExpectedScreen::Dialog(multi.clone()))];
    multi.checked = vec![1];
    want.push(key("1", ExpectedScreen::Dialog(multi.clone())));
    multi.checked = vec![1, 3];
    want.push(key("3", ExpectedScreen::Dialog(multi.clone())));
    multi.cursor = Some(QuestionCursor::Option(4));
    multi.text_active = true;
    want.push(Step {
        input: Input::Keys(vec!["down".into(); 3]),
        expected: ExpectedScreen::Dialog(multi.clone()),
        only_if: None,
    });
    multi.text = "Pepper".into();
    multi.checked.push(4);
    want.push(text("Pepper", multi.clone()));
    multi.cursor = Some(QuestionCursor::Submit);
    want.push(key("down", ExpectedScreen::Dialog(multi)));
    let mut review = dialog(QuestionStep::ClaudeReview, QuestionCursor::Option(1));
    review.review_answers = vec!["Large".into(), "Cheese, Basil, Pepper".into()];
    want.push(key("enter", ExpectedScreen::Dialog(review)));
    want.push(key("1", ExpectedScreen::Gone));
    assert_eq!(plan(pending.kind, &pending, &screen, &json!({"answers":[{"index":1,"options":[3,1],"text":"Pepper"},{"index":0,"options":[2]}]})).unwrap(), want);
}
fn one_claude(multi: bool) -> (PendingQuestion, QuestionScreen) {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../compatibility/phase2/fixtures");
    let rows =
        std::fs::read_to_string(root.join("claude-question-one-transcript-pending.jsonl")).unwrap();
    let records: Vec<Value> = rows
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let spool: Vec<Value> = std::fs::read_to_string(root.join("claude-question-one-spool.jsonl"))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let mut pending = question_match::pending_claude_spool_questions(&spool, &records).remove(0);
    pending.questions[0].multi_select = multi;
    let mut raw = std::fs::read_to_string(root.join("claude-question-one-screen.txt")).unwrap();
    if multi {
        raw = raw
            .replace(" ☐ Color", "←  ☐ Color  ✔ Submit  →")
            .replace("1. Red", "1. [ ] Red")
            .replace("2. Blue", "2. [ ] Blue")
            .replace("3. Type something.", "3. [ ] Type something\n     Submit");
    }
    let screen = question_screen::parse(pending.kind, &raw, &pending).unwrap();
    (pending, screen)
}
#[test]
fn claude_one_question_single_submits_directly_but_multi_requires_review() {
    let (pending, screen) = one_claude(false);
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[2]}]})
        )
        .unwrap(),
        vec![key("2", ExpectedScreen::Gone)]
    );
    let mut other = dialog(QuestionStep::ClaudeQuestion(0), QuestionCursor::Option(3));
    other.text_active = true;
    let mut typed = other.clone();
    typed.text = "Green".into();
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[],"text":"Green"}]})
        )
        .unwrap(),
        vec![
            Step {
                input: Input::Keys(vec!["down".into(); 2]),
                expected: ExpectedScreen::Dialog(other),
                only_if: None
            },
            text("Green", typed),
            key("enter", ExpectedScreen::Gone)
        ]
    );
    let (pending, screen) = one_claude(true);
    let mut checked = dialog(QuestionStep::ClaudeQuestion(0), QuestionCursor::Option(1));
    checked.checked = vec![2];
    let mut submit = checked.clone();
    submit.cursor = Some(QuestionCursor::Submit);
    let mut review = dialog(QuestionStep::ClaudeReview, QuestionCursor::Option(1));
    review.review_answers = vec!["Blue".into()];
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[2]}]})
        )
        .unwrap(),
        vec![
            key("2", ExpectedScreen::Dialog(checked)),
            Step {
                input: Input::Keys(vec!["down".into(); 3]),
                expected: ExpectedScreen::Dialog(submit),
                only_if: None
            },
            key("enter", ExpectedScreen::Dialog(review)),
            key("1", ExpectedScreen::Gone)
        ]
    );
}
#[test]
fn claude_single_text_moves_to_next_tab_before_review() {
    let (pending, screen) = fixture(QuestionKind::ClaudeAsk, "claude-question-single");
    let mut other = dialog(QuestionStep::ClaudeQuestion(0), QuestionCursor::Option(3));
    other.text_active = true;
    let mut typed = other.clone();
    typed.text = "Medium".into();
    let steps = plan(
        pending.kind,
        &pending,
        &screen,
        &json!({"answers":[{"index":0,"options":[],"text":"Medium"},{"index":1,"options":[2]}]}),
    )
    .unwrap();
    assert_eq!(
        &steps[..3],
        &[
            Step {
                input: Input::Keys(vec!["down".into(); 2]),
                expected: ExpectedScreen::Dialog(other),
                only_if: None
            },
            text("Medium", typed),
            key(
                "enter",
                ExpectedScreen::Dialog(dialog(
                    QuestionStep::ClaudeQuestion(1),
                    QuestionCursor::Option(1)
                ))
            )
        ]
    );
}
fn sync_dialog(index: usize, cursor: QuestionCursor, unanswered: usize) -> ExpectedDialog {
    let mut d = dialog(QuestionStep::CodexQuestion { index, total: 2 }, cursor);
    d.unanswered = Some(unanswered..=unanswered);
    if d.cursor == Some(QuestionCursor::Notes) {
        d.notes_option = Some(1);
    }
    d
}
#[test]
fn codex_digits_advance_then_conditionally_submit_last_question() {
    let (pending, screen) = fixture(QuestionKind::CodexPlan, "codex-sync-first");
    let mut last = sync_dialog(1, QuestionCursor::Option(3), 1);
    last.unanswered = Some(0..=1);
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[1]},{"index":1,"options":[3]}]})
        )
        .unwrap(),
        vec![
            key(
                "1",
                ExpectedScreen::Dialog(sync_dialog(1, QuestionCursor::Option(1), 1))
            ),
            key("3", ExpectedScreen::GoneOr(last.clone())),
            Step {
                input: Input::Keys(vec!["enter".into()]),
                expected: ExpectedScreen::Gone,
                only_if: Some(last)
            }
        ]
    );
}
#[test]
fn codex_notes_preserve_selected_option_and_advance_or_submit() {
    let (pending, screen) = fixture(QuestionKind::CodexPlan, "codex-sync-first");
    let mut first = sync_dialog(0, QuestionCursor::Option(2), 2);
    let mut want = vec![key("down", ExpectedScreen::Dialog(first.clone()))];
    first.cursor = Some(QuestionCursor::Notes);
    first.text_active = true;
    first.notes_option = Some(2);
    want.push(key("tab", ExpectedScreen::Dialog(first.clone())));
    first.text = "Medium".into();
    want.push(text("Medium", first));
    want.push(key(
        "enter",
        ExpectedScreen::Dialog(sync_dialog(1, QuestionCursor::Option(1), 1)),
    ));
    let mut last = sync_dialog(1, QuestionCursor::Notes, 1);
    last.text_active = true;
    want.push(key("tab", ExpectedScreen::Dialog(last.clone())));
    last.text = "Cheese, Basil".into();
    want.push(text("Cheese, Basil", last));
    want.push(key("enter", ExpectedScreen::Gone));
    assert_eq!(plan(pending.kind,&pending,&screen,&json!({"answers":[{"index":0,"options":[2],"text":"Medium"},{"index":1,"options":[1],"text":"Cheese, Basil"}]})).unwrap(),want);
}
#[test]
fn invalid_answer_shapes_cardinality_indices_numbers_and_text_are_rejected() {
    let (pending, screen) = one_claude(false);
    for params in [
        json!({}),
        json!({"answers":null}),
        json!({"answers":{}}),
        json!({"answers":[]}),
        json!({"skip":true}),
        json!({"skip":false,"answers":[{"index":0,"options":[1]}]}),
        json!({"answers":[null]}),
        json!({"answers":[{"index":0}]}),
        json!({"answers":[{"index":-1,"options":[1]}]}),
        json!({"answers":[{"index":0.5,"options":[1]}]}),
        json!({"answers":[{"index":1,"options":[1]}]}),
        json!({"answers":[{"index":0,"options":[1]},{"index":0,"options":[2]}]}),
        json!({"answers":[{"index":0,"options":null}]}),
        json!({"answers":[{"index":0,"options":[0]}]}),
        json!({"answers":[{"index":0,"options":[3]}]}),
        json!({"answers":[{"index":0,"options":[-1]}]}),
        json!({"answers":[{"index":0,"options":["1"]}]}),
        json!({"answers":[{"index":0,"options":[1,2]}]}),
        json!({"answers":[{"index":0,"options":[]}]}),
        json!({"answers":[{"index":0,"options":[],"text":""}]}),
        json!({"answers":[{"index":0,"options":[],"text":"\n"}]}),
        json!({"answers":[{"index":0,"options":[],"text":"\u{1b}"}]}),
        json!({"answers":[{"index":0,"options":[],"text":null}]}),
        json!({"answers":[{"index":0,"options":[],"text":3}]}),
        json!({"answers":[{"index":0,"options":[],"text":"x".repeat(501)}]}),
    ] {
        assert_eq!(
            plan(pending.kind, &pending, &screen, &params),
            Err("invalid"),
            "{params}"
        );
    }
    let (pending, screen) = one_claude(true);
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[1,1]}]})
        ),
        Err("invalid")
    );
    let (pending, screen) = fixture(QuestionKind::CodexPlan, "codex-sync-first");
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[],"text":"Medium"},{"index":1,"options":[1]}]})
        ),
        Err("invalid")
    );
    let (pending, screen) = fixture(QuestionKind::CodexAsync, "codex-async-queued");
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"skip":true,"answers":[]})
        ),
        Err("invalid")
    );
}
#[test]
fn text_limit_counts_unicode_characters_and_rejects_all_controls() {
    let (pending, screen) = one_claude(false);
    assert!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[],"text":"猫".repeat(500)}]})
        )
        .is_ok()
    );
    for c in (0u8..=127)
        .map(char::from)
        .chain(['\u{85}', '\u{9f}'])
        .filter(|c| c.is_control())
    {
        assert_eq!(
            plan(
                pending.kind,
                &pending,
                &screen,
                &json!({"answers":[{"index":0,"options":[],"text":format!("a{c}b")}]})
            ),
            Err("invalid")
        );
    }
}

#[test]
fn checks_accept_captures_but_reject_wrong_notes_selection_review_or_checks() {
    let (pending, first) = fixture(QuestionKind::CodexPlan, "codex-sync-first");
    let steps = plan(pending.kind,&pending,&first,&json!({"answers":[{"index":0,"options":[1]},{"index":1,"options":[1],"text":"Cheese, Basil"}]})).unwrap();
    let (_, last) = fixture(QuestionKind::CodexPlan, "codex-sync-last");
    assert!(steps[0].expected.matches(Some(&last)));
    assert!(!steps[0].expected.matches(Some(&first)));
    assert!(!steps[0].expected.matches(None));
    let (_, notes) = fixture(QuestionKind::CodexPlan, "codex-sync-notes");
    assert!(steps[2].expected.matches(Some(&notes)));
    let mut wrong = notes.clone();
    wrong.highlighted = Some(2);
    assert!(!steps[2].expected.matches(Some(&wrong)));
    wrong = notes.clone();
    wrong.free_text.as_mut().unwrap().text = "Cheese".into();
    assert!(!steps[2].expected.matches(Some(&wrong)));
    wrong = notes.clone();
    wrong.unanswered = Some(2);
    assert!(!steps[2].expected.matches(Some(&wrong)));
    let (pending, screen) = fixture(QuestionKind::ClaudeAsk, "claude-question-single");
    let steps = plan(
        pending.kind,
        &pending,
        &screen,
        &json!({"answers":[{"index":0,"options":[2]},{"index":1,"options":[1,3],"text":"Pepper"}]}),
    )
    .unwrap();
    let (_, review) = fixture(QuestionKind::ClaudeAsk, "claude-question-review");
    let expected = &steps[steps.len() - 2].expected;
    assert!(expected.matches(Some(&review)));
    let mut wrong = review.clone();
    wrong.review_answers[0] = "Small".into();
    assert!(!expected.matches(Some(&wrong)));
    wrong = review.clone();
    wrong.cursor = Some(QuestionCursor::Option(2));
    assert!(!expected.matches(Some(&wrong)));
    assert!(steps.last().unwrap().expected.matches(None));
    assert!(!steps.last().unwrap().expected.matches(Some(&review)));
    // The captured toggled screen shows the exact checkbox set before typing.
    let (_, multi) = fixture(QuestionKind::ClaudeAsk, "claude-question-multi");
    assert!(steps[2].expected.matches(Some(&multi)));
    let mut wrong = multi.clone();
    wrong.options[1].checked = Some(true);
    assert!(!steps[2].expected.matches(Some(&wrong)));
}
#[test]
fn last_codex_digit_accepts_only_disappearance_or_the_requested_last_selection() {
    let (pending, screen) = fixture(QuestionKind::CodexPlan, "codex-sync-first");
    let steps = plan(
        pending.kind,
        &pending,
        &screen,
        &json!({"answers":[{"index":0,"options":[1]},{"index":1,"options":[1]}]}),
    )
    .unwrap();
    let (_, mut last) = fixture(QuestionKind::CodexPlan, "codex-sync-last");
    assert!(steps[1].expected.matches(None));
    assert!(steps[1].expected.matches(Some(&last)));
    assert!(steps[2].only_if.as_ref().unwrap().matches(&last));
    last.unanswered = Some(0);
    assert!(steps[1].expected.matches(Some(&last)));
    last.cursor = Some(QuestionCursor::Option(2));
    assert!(!steps[1].expected.matches(Some(&last)));
    assert!(!steps[2].only_if.as_ref().unwrap().matches(&last));
    assert!(!steps[1].expected.matches(Some(&screen)));
    // An all-answered last picker must still parse for the conditional Enter.
    let raw = include_str!("../../../compatibility/phase2/fixtures/codex-sync-last-screen.txt")
        .replace("(1 unanswered)", "(0 unanswered)");
    assert!(question_screen::parse(pending.kind, &raw, &pending).is_some());
}
#[test]
fn nonfresh_or_incompatible_requests_are_never_planned() {
    let (pending, screen) = fixture(QuestionKind::ClaudeAsk, "claude-question-multi");
    let params = json!({"answers":[{"index":0,"options":[1]},{"index":1,"options":[1]}]});
    assert_eq!(
        plan(pending.kind, &pending, &screen, &params),
        Err("in_progress")
    );
    let (_, fresh) = fixture(QuestionKind::ClaudeAsk, "claude-question-single");
    assert_eq!(
        plan(QuestionKind::CodexPlan, &pending, &fresh, &params),
        Err("mismatch")
    );
    let mut secret = pending.clone();
    secret.questions[0].secret = true;
    assert_eq!(plan(secret.kind, &secret, &fresh, &params), Err("mismatch"));
    let (pending, screen) = fixture(QuestionKind::CodexAsync, "codex-async-queued");
    let mut multiple = pending.clone();
    multiple.questions.push(multiple.questions[0].clone());
    assert_eq!(
        plan(multiple.kind, &multiple, &screen, &json!({"skip":true})),
        Err("mismatch")
    );
    let (pending, screen) = one_claude(false);
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[1],"text":"extra"}]})
        ),
        Err("mismatch")
    );
    let mut no_text = pending.clone();
    no_text.questions[0].allows_other = false;
    assert_eq!(
        plan(
            no_text.kind,
            &no_text,
            &screen,
            &json!({"answers":[{"index":0,"options":[],"text":"Green"}]})
        ),
        Err("mismatch")
    );
    let (_, alien) = fixture(QuestionKind::CodexPlan, "codex-sync-first");
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &alien,
            &json!({"answers":[{"index":0,"options":[1]}]})
        ),
        Err("mismatch")
    );
}
#[test]
fn missing_or_duplicate_indices_are_invalid_even_with_the_right_answer_count() {
    let (pending, screen) = fixture(QuestionKind::ClaudeAsk, "claude-question-single");
    for params in [
        json!({"answers":[{"index":0,"options":[1]}]}),
        json!({"answers":[{"index":0,"options":[1]},{"index":0,"options":[2]}]}),
        json!({"answers":[{"options":[1]},{"index":1,"options":[2]}]}),
    ] {
        assert_eq!(
            plan(pending.kind, &pending, &screen, &params),
            Err("invalid")
        );
    }
}
#[test]
fn unicode_line_separators_are_not_single_line_answers() {
    let (pending, screen) = one_claude(false);
    for text in ["a\u{2028}b", "a\u{2029}b"] {
        assert_eq!(
            plan(
                pending.kind,
                &pending,
                &screen,
                &json!({"answers":[{"index":0,"options":[],"text":text}]})
            ),
            Err("invalid")
        );
    }
}

#[test]
fn async_option_from_open_picker_does_not_reopen_it() {
    let (pending, screen) = fixture(QuestionKind::CodexAsync, "codex-async-picker");
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[1]}]})
        )
        .unwrap(),
        vec![
            key(
                "1",
                ExpectedScreen::Dialog(dialog(
                    QuestionStep::AsyncPicker(0),
                    QuestionCursor::Option(1)
                ))
            ),
            key("enter", ExpectedScreen::Gone)
        ]
    );
}
#[test]
fn codex_text_only_uses_none_of_the_above_and_notes() {
    let (pending, screen) = fixture(QuestionKind::CodexPlan, "codex-sync-first");
    let mut selected = sync_dialog(0, QuestionCursor::Option(3), 2);
    let mut want = vec![Step {
        input: Input::Keys(vec!["down".into(); 2]),
        expected: ExpectedScreen::Dialog(selected.clone()),
        only_if: None,
    }];
    selected.cursor = Some(QuestionCursor::Notes);
    selected.notes_option = Some(3);
    selected.text_active = true;
    want.push(key("tab", ExpectedScreen::Dialog(selected.clone())));
    selected.text = "Medium".into();
    want.push(text("Medium", selected));
    want.push(key(
        "enter",
        ExpectedScreen::Dialog(sync_dialog(1, QuestionCursor::Option(1), 1)),
    ));
    let steps = plan(
        pending.kind,
        &pending,
        &screen,
        &json!({"answers":[{"index":0,"options":[3],"text":"Medium"},{"index":1,"options":[1]}]}),
    )
    .unwrap();
    assert_eq!(&steps[..4], &want);
}

#[test]
fn codex_notes_can_choose_an_option_above_the_starting_highlight() {
    let (pending, mut screen) = fixture(QuestionKind::CodexPlan, "codex-sync-first");
    screen.cursor = Some(QuestionCursor::Option(2));
    screen.highlighted = Some(2);
    let steps=plan(pending.kind,&pending,&screen,&json!({"answers":[{"index":0,"options":[1],"text":"Small please"},{"index":1,"options":[1]}]})).unwrap();
    assert_eq!(
        steps[0],
        key(
            "up",
            ExpectedScreen::Dialog(sync_dialog(0, QuestionCursor::Option(1), 2))
        )
    );
}

#[test]
fn one_question_claude_multiselect_plan_checks_review_before_final_submit() {
    let (mut pending, _) = fixture(QuestionKind::ClaudeAsk, "claude-question-single");
    pending.questions.remove(0);
    let raw =
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-multi-screen.txt");
    let screen = question_screen::parse(pending.kind, raw, &pending).unwrap();
    let mut selected = dialog(QuestionStep::ClaudeQuestion(0), QuestionCursor::Option(1));
    selected.checked = vec![1];
    let mut want = vec![key("1", ExpectedScreen::Dialog(selected.clone()))];
    selected.checked = vec![1, 3];
    want.push(key("3", ExpectedScreen::Dialog(selected.clone())));
    selected.cursor = Some(QuestionCursor::Submit);
    want.push(Step {
        input: Input::Keys(vec!["down".into(); 4]),
        expected: ExpectedScreen::Dialog(selected),
        only_if: None,
    });
    let mut review = dialog(QuestionStep::ClaudeReview, QuestionCursor::Option(1));
    review.review_answers = vec!["Cheese, Basil".into()];
    want.push(key("enter", ExpectedScreen::Dialog(review)));
    want.push(key("1", ExpectedScreen::Gone));
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[1,3]}]})
        )
        .unwrap(),
        want
    );
}

#[test]
fn one_question_claude_multiselect_text_auto_checks_and_is_reviewed() {
    let (mut pending, _) = fixture(QuestionKind::ClaudeAsk, "claude-question-single");
    pending.questions.remove(0);
    let raw =
        include_str!("../../../compatibility/phase2/fixtures/claude-question-one-multi-screen.txt");
    let screen = question_screen::parse(pending.kind, raw, &pending).unwrap();
    let mut other = dialog(QuestionStep::ClaudeQuestion(0), QuestionCursor::Option(4));
    other.text_active = true;
    let mut typed = other.clone();
    typed.text = "Pepper".into();
    typed.checked = vec![4];
    let mut submit = typed.clone();
    submit.cursor = Some(QuestionCursor::Submit);
    let mut review = dialog(QuestionStep::ClaudeReview, QuestionCursor::Option(1));
    review.review_answers = vec!["Pepper".into()];
    assert_eq!(
        plan(
            pending.kind,
            &pending,
            &screen,
            &json!({"answers":[{"index":0,"options":[],"text":"Pepper"}]})
        )
        .unwrap(),
        vec![
            Step {
                input: Input::Keys(vec!["down".into(); 3]),
                expected: ExpectedScreen::Dialog(other),
                only_if: None
            },
            text("Pepper", typed),
            key("down", ExpectedScreen::Dialog(submit)),
            key("enter", ExpectedScreen::Dialog(review)),
            key("1", ExpectedScreen::Gone)
        ]
    );
}

#[test]
fn codex_first_notes_live_capture_matches_plan_after_tab() {
    let (mut pending, _) = fixture(QuestionKind::CodexPlan, "codex-sync-first");
    pending.questions[0].options[0].description = Some("Choose Small.".into());
    pending.questions[0].options[1].description = Some("Choose Large.".into());
    let initial =
        include_str!("../../../compatibility/phase2/fixtures/codex-sync-first-screen.txt")
            .replace("Choose a small size.", "Choose Small.")
            .replace("Choose a large size.", "Choose Large.");
    let initial = question_screen::parse(pending.kind, &initial, &pending).unwrap();
    let steps = plan(pending.kind, &pending, &initial,
        &json!({"answers":[{"index":0,"options":[3],"text":"medium please"},{"index":1,"options":[1]}]})).unwrap();
    let notes = question_screen::parse(
        pending.kind,
        include_str!("../../../compatibility/phase2/fixtures/codex-sync-notes-first-screen.txt"),
        &pending,
    )
    .unwrap();
    assert_eq!(steps[1].input, Input::Keys(vec!["tab".into()]));
    assert!(steps[1].expected.matches(Some(&notes)));
}
