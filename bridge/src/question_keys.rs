//! Pure, guarded terminal input plans for question responses.
//!
//! The executor parses each visible screen against the same pending request and
//! checks `expected` after every input. Only the final Codex digit has two valid
//! outcomes: disappearance, or the selected last question awaiting Enter.
use crate::{
    Result,
    question_match::{PendingQuestion, QuestionKind},
    question_screen::{QuestionCursor, QuestionScreen, QuestionStep, claude_has_review},
};
use serde_json::Value;
use std::ops::RangeInclusive;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Input {
    Keys(Vec<String>),
    Text(String),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedDialog {
    pub step: QuestionStep,
    pub cursor: Option<QuestionCursor>,
    pub checked: Vec<u32>,
    pub text: String,
    pub text_active: bool,
    pub unanswered: Option<RangeInclusive<usize>>,
    pub review_answers: Vec<String>,
    pub notes_option: Option<u32>,
}
impl ExpectedDialog {
    /// `screen` must have been parsed against the same PendingQuestion as the plan.
    pub fn matches(&self, screen: &QuestionScreen) -> bool {
        let checked: Vec<_> = screen
            .options
            .iter()
            .filter(|o| o.checked == Some(true))
            .map(|o| o.number)
            .collect();
        self.step == screen.step
            && self.cursor == screen.cursor
            && self.checked == checked
            && compact(&self.text)
                == compact(screen.free_text.as_ref().map_or("", |t| t.text.as_str()))
            && self.text_active == screen.free_text.as_ref().is_some_and(|t| t.active)
            && match (&self.unanswered, screen.unanswered) {
                (Some(range), Some(n)) => range.contains(&n),
                (None, None) => true,
                _ => false,
            }
            && self
                .notes_option
                .is_none_or(|n| screen.highlighted == Some(n))
            && self.review_answers.len() == screen.review_answers.len()
            && self
                .review_answers
                .iter()
                .zip(&screen.review_answers)
                .all(|(a, b)| compact(a) == compact(b))
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpectedScreen {
    Dialog(ExpectedDialog),
    Gone,
    GoneOr(ExpectedDialog),
}
impl ExpectedScreen {
    pub fn matches(&self, screen: Option<&QuestionScreen>) -> bool {
        match (self, screen) {
            (Self::Gone | Self::GoneOr(_), None) => true,
            (Self::Dialog(expected) | Self::GoneOr(expected), Some(screen)) => {
                expected.matches(screen)
            }
            _ => false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub input: Input,
    pub expected: ExpectedScreen,
    /// Skip only if the dialog is gone; otherwise require this exact state before input.
    /// Used solely for the optional Enter after the last Codex Plan digit.
    pub only_if: Option<ExpectedDialog>,
}
fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}
#[derive(Debug)]
struct Answer {
    options: Vec<u32>,
    text: Option<String>,
}
fn answers(pending: &PendingQuestion, params: &Value) -> Result<Option<Vec<Answer>>> {
    if params.get("skip").is_some() {
        if params["skip"] != true
            || params.get("answers").is_some()
            || pending.kind != QuestionKind::CodexAsync
        {
            return Err("invalid");
        }
        return Ok(None);
    }
    let values = params["answers"].as_array().ok_or("invalid")?;
    if values.len() != pending.questions.len() {
        return Err("invalid");
    }
    let mut result: Vec<Option<Answer>> = (0..values.len()).map(|_| None).collect();
    for value in values {
        let index = value["index"]
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .ok_or("invalid")?;
        let q = pending.questions.get(index).ok_or("invalid")?;
        if result[index].is_some() {
            return Err("invalid");
        }
        let mut options = value["options"]
            .as_array()
            .ok_or("invalid")?
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or("invalid")
            })
            .collect::<Result<Vec<_>>>()?;
        let max = q.options.len()
            + usize::from(pending.kind == QuestionKind::CodexPlan && q.allows_other);
        if options.iter().any(|&n| n == 0 || n as usize > max)
            || (!q.multi_select && options.len() > 1)
        {
            return Err("invalid");
        }
        options.sort_unstable();
        if options.windows(2).any(|w| w[0] == w[1]) {
            return Err("invalid");
        }
        let text = match value.get("text") {
            Some(v) => {
                let text = v.as_str().ok_or("invalid")?;
                if text.chars().count() > 500
                    || text
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
                {
                    return Err("invalid");
                }
                if text.is_empty() {
                    None
                } else {
                    Some(text.to_owned())
                }
            }
            None => None,
        };
        if (options.is_empty() && text.is_none())
            || (pending.kind == QuestionKind::CodexPlan && options.len() != 1)
        {
            return Err("invalid");
        }
        if text.is_some()
            && (!q.allows_other
                || (pending.kind != QuestionKind::CodexPlan
                    && !q.multi_select
                    && !options.is_empty()))
        {
            return Err("mismatch");
        }
        result[index] = Some(Answer { options, text });
    }
    result
        .into_iter()
        .map(|a| a.ok_or("invalid"))
        .collect::<Result<Vec<_>>>()
        .map(Some)
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
fn push(steps: &mut Vec<Step>, input: Input, expected: ExpectedScreen) {
    steps.push(Step {
        input,
        expected,
        only_if: None,
    });
}
fn key(steps: &mut Vec<Step>, key: impl Into<String>, expected: ExpectedScreen) {
    push(steps, Input::Keys(vec![key.into()]), expected);
}
fn down(steps: &mut Vec<Step>, count: u32, expected: &ExpectedDialog) {
    if count > 0 {
        push(
            steps,
            Input::Keys(vec!["down".into(); count as usize]),
            ExpectedScreen::Dialog(expected.clone()),
        );
    }
}
fn next_claude(pending: &PendingQuestion, index: usize, answers: &[Answer]) -> ExpectedScreen {
    if index + 1 < pending.questions.len() {
        ExpectedScreen::Dialog(dialog(
            QuestionStep::ClaudeQuestion(index + 1),
            QuestionCursor::Option(1),
        ))
    } else if !claude_has_review(pending) {
        ExpectedScreen::Gone
    } else {
        let mut review = dialog(QuestionStep::ClaudeReview, QuestionCursor::Option(1));
        review.review_answers = pending
            .questions
            .iter()
            .zip(answers)
            .map(|(q, a)| {
                let mut labels: Vec<_> = a
                    .options
                    .iter()
                    .map(|n| q.options[*n as usize - 1].label.clone())
                    .collect();
                labels.extend(a.text.iter().cloned());
                labels.join(", ")
            })
            .collect();
        ExpectedScreen::Dialog(review)
    }
}
fn claude(
    pending: &PendingQuestion,
    screen: &QuestionScreen,
    answers: &[Answer],
) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    for (index, (q, answer)) in pending.questions.iter().zip(answers).enumerate() {
        let mut state = dialog(
            QuestionStep::ClaudeQuestion(index),
            QuestionCursor::Option(1),
        );
        if index == 0 {
            state.cursor = screen.cursor.clone();
        }
        let mut cursor = match state.cursor {
            Some(QuestionCursor::Option(n)) => n,
            _ => return Err("mismatch"),
        };
        if q.multi_select {
            for &number in &answer.options {
                state.checked.push(number);
                // Digits toggle without moving the current cursor (captured).
                // observed 2026-10-22: one-question multi-select toggles use the same tabbed flow.
                key(
                    &mut steps,
                    number.to_string(),
                    ExpectedScreen::Dialog(state.clone()),
                );
            }
        } else if let Some(&number) = answer.options.first() {
            key(
                &mut steps,
                number.to_string(),
                next_claude(pending, index, answers),
            );
            continue;
        }
        let other = u32::try_from(q.options.len() + 1).map_err(|_| "mismatch")?;
        if let Some(text) = &answer.text {
            state.cursor = Some(QuestionCursor::Option(other));
            state.text_active = true;
            // observed 2026-10-22: single-select text uses down, text, Enter, both inside tabs and in a bare one-question picker.
            down(
                &mut steps,
                other.checked_sub(cursor).ok_or("mismatch")?,
                &state,
            );
            cursor = other;
            state.text = text.clone();
            if q.multi_select {
                // observed 2026-10-22: typing into the free-text row auto-checks it.
                state.checked.push(other);
            }
            push(
                &mut steps,
                Input::Text(text.clone()),
                ExpectedScreen::Dialog(state.clone()),
            );
        }
        if q.multi_select {
            state.cursor = Some(QuestionCursor::Submit);
            // observed 2026-10-22: Submit on one-question multi-select leads to review.
            down(
                &mut steps,
                (other + 1).checked_sub(cursor).ok_or("mismatch")?,
                &state,
            );
        }
        // observed 2026-10-22: text Enter advances/submits single-select; multi-select Submit Enter opens review.
        key(&mut steps, "enter", next_claude(pending, index, answers));
    }
    if claude_has_review(pending) {
        key(&mut steps, "1", ExpectedScreen::Gone);
    }
    Ok(steps)
}
fn codex_dialog(index: usize, total: usize, cursor: QuestionCursor) -> ExpectedDialog {
    let mut state = dialog(QuestionStep::CodexQuestion { index, total }, cursor);
    state.unanswered = Some(total - index..=total - index);
    state
}
fn codex(
    pending: &PendingQuestion,
    screen: &QuestionScreen,
    answers: &[Answer],
) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    let total = pending.questions.len();
    for (index, answer) in answers.iter().enumerate() {
        let number = answer.options[0];
        let mut state = codex_dialog(index, total, QuestionCursor::Option(number));
        let next = if index + 1 == total {
            ExpectedScreen::Gone
        } else {
            ExpectedScreen::Dialog(codex_dialog(index + 1, total, QuestionCursor::Option(1)))
        };
        if let Some(text) = &answer.text {
            let cursor = if index == 0 {
                match screen.cursor {
                    Some(QuestionCursor::Option(n)) => n,
                    _ => return Err("mismatch"),
                }
            } else {
                1
            };
            if number < cursor {
                // unverified: use the matching up key when an otherwise fresh picker has a lower highlight.
                push(
                    &mut steps,
                    Input::Keys(vec!["up".into(); (cursor - number) as usize]),
                    ExpectedScreen::Dialog(state.clone()),
                );
            } else {
                down(&mut steps, number - cursor, &state);
            }
            state.cursor = Some(QuestionCursor::Notes);
            state.notes_option = Some(number);
            state.text_active = true;
            key(&mut steps, "tab", ExpectedScreen::Dialog(state.clone()));
            state.text = text.clone();
            push(
                &mut steps,
                Input::Text(text.clone()),
                ExpectedScreen::Dialog(state),
            );
            // observed 2026-10-22: notes Enter on a non-last question advances to the next question.
            key(&mut steps, "enter", next);
        } else if index + 1 < total {
            key(&mut steps, number.to_string(), next);
        } else {
            // observed 2026-10-22: the last digit submits immediately; the only_if Enter is skipped.
            // unverified: retain the guarded fallback if the last picker instead remains selected.
            state.unanswered = Some(0..=1);
            key(
                &mut steps,
                number.to_string(),
                ExpectedScreen::GoneOr(state.clone()),
            );
            steps.push(Step {
                input: Input::Keys(vec!["enter".into()]),
                expected: ExpectedScreen::Gone,
                only_if: Some(state),
            });
        }
    }
    Ok(steps)
}
fn asynchronous(
    pending: &PendingQuestion,
    screen: &QuestionScreen,
    answers: Option<&[Answer]>,
) -> Result<Vec<Step>> {
    let mut steps = Vec::new();
    if screen.step == QuestionStep::AsyncQueued {
        // observed 2026-10-22: shift+left opens the queued picker (including a 20s countdown).
        key(
            &mut steps,
            "shift+left",
            ExpectedScreen::Dialog(dialog(
                QuestionStep::AsyncPicker(0),
                QuestionCursor::Option(1),
            )),
        );
    }
    let Some(answers) = answers else {
        // observed 2026-10-22: ctrl+] closes the open picker and confirms async skip.
        key(&mut steps, "ctrl+]", ExpectedScreen::Gone);
        return Ok(steps);
    };
    let answer = &answers[0];
    let other = u32::try_from(pending.questions[0].options.len() + 1).map_err(|_| "mismatch")?;
    let number = answer.options.first().copied().unwrap_or(other);
    let mut state = dialog(QuestionStep::AsyncPicker(0), QuestionCursor::Option(number));
    state.text_active = answer.text.is_some();
    // observed 2026-10-22: Other digit, typed text, then Enter delivers Green and resolves.
    // observed in phase 1 (2026-10-13): a digit selected an ordinary option and Enter submitted it.
    key(
        &mut steps,
        number.to_string(),
        ExpectedScreen::Dialog(state.clone()),
    );
    if let Some(text) = &answer.text {
        state.text = text.clone();
        push(
            &mut steps,
            Input::Text(text.clone()),
            ExpectedScreen::Dialog(state),
        );
    }
    key(&mut steps, "enter", ExpectedScreen::Gone);
    Ok(steps)
}
/// Validate the answers/skip in interaction.respond params, then plan from a fresh parsed screen.
/// The service checks request_id and the live occupant before executing this plan.
/// Indices are zero-based; terminal option numbers are one-based. No I/O occurs here.
pub fn plan(
    kind: QuestionKind,
    pending: &PendingQuestion,
    screen: &QuestionScreen,
    params: &Value,
) -> Result<Vec<Step>> {
    if kind != pending.kind
        || pending.questions.is_empty()
        || pending.questions.iter().any(|q| q.secret)
        || (kind == QuestionKind::CodexAsync && pending.questions.len() != 1)
    {
        return Err("mismatch");
    }
    let compatible = match (kind, &screen.step) {
        (QuestionKind::ClaudeAsk, QuestionStep::ClaudeQuestion(_) | QuestionStep::ClaudeReview) => {
            true
        }
        (QuestionKind::CodexPlan, QuestionStep::CodexQuestion { total, .. }) => {
            *total == pending.questions.len()
        }
        (QuestionKind::CodexAsync, QuestionStep::AsyncQueued | QuestionStep::AsyncPicker(0)) => {
            true
        }
        _ => false,
    };
    if !compatible {
        return Err("mismatch");
    }
    let answers = answers(pending, params)?;
    if !screen.is_fresh(pending) {
        return Err("in_progress");
    }
    match kind {
        QuestionKind::ClaudeAsk => claude(pending, screen, answers.as_deref().ok_or("invalid")?),
        QuestionKind::CodexPlan => codex(pending, screen, answers.as_deref().ok_or("invalid")?),
        QuestionKind::CodexAsync => asynchronous(pending, screen, answers.as_deref()),
    }
}
#[cfg(test)]
mod tests;
