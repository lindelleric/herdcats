//! Screen parsing anchored to structured pending questions. All dialog rows enter the fingerprint.
use crate::question_match::{PendingQuestion, QuestionKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionStep {
    /// A single Claude question submits directly and has no review step.
    ClaudeQuestion(usize),
    ClaudeReview,
    CodexQuestion {
        index: usize,
        total: usize,
    },
    AsyncQueued,
    AsyncPicker(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionCursor {
    Option(u32),
    Submit,
    Notes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScreenOption {
    pub number: u32,
    pub label: String,
    pub description: Option<String>,
    pub checked: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreeText {
    pub active: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionScreen {
    pub step: QuestionStep,
    pub options: Vec<ScreenOption>,
    pub cursor: Option<QuestionCursor>,
    pub free_text: Option<FreeText>,
    pub countdown: Option<String>,
    pub queued_count: Option<usize>,
    /// Lossless length-delimited rows, including whitespace, selection, and footer.
    pub fingerprint: Vec<u8>,
}

fn fingerprint(rows: &[&str]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for row in rows {
        bytes.extend_from_slice(&(row.len() as u64).to_le_bytes());
        bytes.extend_from_slice(row.as_bytes());
    }
    bytes
}

fn numbered(row: &str) -> Option<(u32, &str, bool)> {
    let row = row.trim();
    let selected = row.starts_with(['❯', '›']);
    let row = row.trim_start_matches(['❯', '›']).trim_start();
    let (n, label) = row.split_once(". ")?;
    if !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((n.parse().ok()?, label.trim(), selected))
}

fn rows_equal(rows: &[&str], expected: &str) -> bool {
    rows.iter()
        .flat_map(|row| row.chars())
        .filter(|c| !c.is_whitespace())
        .eq(expected.chars().filter(|c| !c.is_whitespace()))
}

fn submit(row: &str) -> bool {
    row.trim().trim_start_matches('❯').trim() == "Submit"
}

fn notes_text(row: &str) -> Option<&str> {
    let row = row.trim();
    if row == "›" {
        Some("")
    } else {
        row.strip_prefix("› ")
    }
}

// Consume visual continuations, keeping control rows outside the text span.
fn option_end(rows: &[&str], start: usize, limit: usize) -> usize {
    (start + 1..limit)
        .find(|&i| {
            numbered(rows[i]).is_some()
                || submit(rows[i])
                || rule(rows[i])
                || notes_text(rows[i]).is_some()
        })
        .unwrap_or(limit)
}

fn result(step: QuestionStep) -> QuestionScreen {
    QuestionScreen {
        step,
        options: Vec::new(),
        cursor: None,
        free_text: None,
        countdown: None,
        queued_count: None,
        fingerprint: Vec::new(),
    }
}

fn cursor(dialog: &mut QuestionScreen, value: QuestionCursor) -> Option<()> {
    if dialog.cursor.replace(value).is_some() {
        None
    } else {
        Some(())
    }
}

fn rule(row: &str) -> bool {
    let row = row.trim();
    !row.is_empty() && row.chars().all(|c| c == '─')
}

fn tab_matches(row: &str, pending: &PendingQuestion) -> bool {
    if pending.questions.len() == 1 {
        let Some(header) = pending.questions[0].header.as_deref() else {
            return false;
        };
        return row
            .trim()
            .strip_prefix('☐')
            .or_else(|| row.trim().strip_prefix('☒'))
            .is_some_and(|rest| rest.trim() == header);
    }
    let mut rest = row.trim().strip_prefix('←').unwrap_or("").trim();
    for q in &pending.questions {
        let Some(header) = q.header.as_deref() else {
            return false;
        };
        let Some(after) = rest.strip_prefix('☐').or_else(|| rest.strip_prefix('☒')) else {
            return false;
        };
        let Some(after) = after.trim_start().strip_prefix(header) else {
            return false;
        };
        if !after.starts_with(char::is_whitespace) {
            return false;
        }
        rest = after.trim_start();
    }
    rest == "✔ Submit  →"
}

fn claude(rows: &[&str], pending: &PendingQuestion) -> Option<QuestionScreen> {
    if !tab_matches(rows[0], pending) {
        return None;
    }
    let title = rows
        .iter()
        .position(|row| !row.trim().is_empty() && *row != rows[0])?;
    if rows[title].trim() == "Review your answers" {
        return (pending.questions.len() > 1)
            .then(|| review(rows, title, pending))
            .flatten();
    }
    let first_option = (title + 1..rows.len()).find(|&i| numbered(rows[i]).is_some())?;
    let matches: Vec<_> = pending
        .questions
        .iter()
        .enumerate()
        .filter(|(_, q)| rows_equal(&rows[title..first_option], &q.text))
        .collect();
    let [(index, q)] = matches.as_slice() else {
        return None;
    };
    let mut dialog = result(QuestionStep::ClaudeQuestion(*index));
    let divider = (title + 1..rows.len()).find(|&i| rule(rows[i]))?;
    let mut submitted = false;
    let mut i = first_option;
    while i < divider {
        if rows[i].trim().is_empty() {
            i += 1;
            continue;
        }
        if rows[i].trim().trim_start_matches('❯').trim() == "Submit" && q.multi_select {
            if submitted || dialog.options.len() != q.options.len() + usize::from(q.allows_other) {
                return None;
            }
            submitted = true;
            if rows[i].contains('❯') {
                cursor(&mut dialog, QuestionCursor::Submit)?;
            }
            i += 1;
            continue;
        }
        let (number, mut label, selected) = numbered(rows[i])?;
        if number as usize != dialog.options.len() + 1 {
            return None;
        }
        let checked = if q.multi_select {
            let (state, after) = if let Some(after) = label.strip_prefix("[✔] ") {
                (true, after)
            } else {
                (false, label.strip_prefix("[ ] ")?)
            };
            label = after;
            Some(state)
        } else {
            None
        };
        let end = option_end(rows, i, divider);
        let mut description = None;
        if let Some(option) = q.options.get(dialog.options.len()) {
            if label != option.label {
                return None;
            }
            if let Some(expected) = &option.description {
                if !rows_equal(&rows[i + 1..end], expected) {
                    return None;
                }
                description = Some(expected.clone());
            } else if !rows_equal(&rows[i + 1..end], "") {
                return None;
            }
        } else {
            if !rows_equal(&rows[i + 1..end], "") {
                return None;
            }
            if dialog.options.len() != q.options.len() || !q.allows_other {
                return None;
            }
            let placeholder = if q.multi_select {
                "Type something"
            } else {
                "Type something."
            };
            let typed = label != placeholder;
            dialog.free_text = Some(FreeText {
                active: selected || checked == Some(true),
                text: if typed { label.into() } else { String::new() },
            });
        }
        if selected {
            cursor(&mut dialog, QuestionCursor::Option(number))?;
        }
        dialog.options.push(ScreenOption {
            number,
            label: label.into(),
            description,
            checked,
        });
        i = end;
    }
    if q.multi_select && !submitted {
        return None;
    }
    if dialog.options.len() != q.options.len() + usize::from(q.allows_other) {
        return None;
    }
    let after: Vec<_> = rows[divider + 1..]
        .iter()
        .filter(|r| !r.trim().is_empty())
        .collect();
    let (number, label, _) = numbered(after.first()?)?;
    if number as usize != dialog.options.len() + 1 || label != "Chat about this" {
        return None;
    }
    if after.len() != 2 || !after[1].contains("Esc to cancel") {
        return None;
    }
    Some(dialog)
}

fn review(rows: &[&str], title: usize, pending: &PendingQuestion) -> Option<QuestionScreen> {
    let body: Vec<_> = rows[title + 1..]
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    let mut i = 0;
    for q in &pending.questions {
        let first = body.get(i)?.strip_prefix("● ")?;
        i += 1;
        let start = i;
        while body
            .get(i)
            .is_some_and(|r| !r.starts_with(['●', '→']) && *r != "Ready to submit your answers?")
        {
            i += 1;
        }
        let mut question_rows = vec![first];
        question_rows.extend_from_slice(&body[start..i]);
        if !rows_equal(&question_rows, &q.text) {
            return None;
        }
        let first = body.get(i)?.strip_prefix('→')?.trim();
        i += 1;
        let start = i;
        while body
            .get(i)
            .is_some_and(|r| !r.starts_with(['●', '→']) && *r != "Ready to submit your answers?")
        {
            i += 1;
        }
        let mut answer_rows = vec![first];
        answer_rows.extend_from_slice(&body[start..i]);
        if rows_equal(&answer_rows, "") {
            return None;
        }
    }
    if *body.get(i)? != "Ready to submit your answers?" || body.len() != i + 3 {
        return None;
    }
    let mut dialog = result(QuestionStep::ClaudeReview);
    for (offset, expected) in ["Submit answers", "Cancel"].iter().enumerate() {
        let (number, label, selected) = numbered(body[i + 1 + offset])?;
        if number as usize != offset + 1 || label != *expected {
            return None;
        }
        if selected {
            cursor(&mut dialog, QuestionCursor::Option(number))?;
        }
        dialog.options.push(ScreenOption {
            number,
            label: label.into(),
            description: None,
            checked: None,
        });
    }
    Some(dialog)
}

fn sync(rows: &[&str], pending: &PendingQuestion) -> Option<QuestionScreen> {
    let header = rows[0].trim().strip_prefix("Question ")?;
    let (position, unanswered) = header.split_once(" (")?;
    let remaining: usize = unanswered.strip_suffix(" unanswered)")?.parse().ok()?;
    let (index, total) = position.split_once('/')?;
    let index: usize = index.parse().ok()?;
    let total: usize = total.parse().ok()?;
    if total != pending.questions.len()
        || index == 0
        || index > total
        || remaining == 0
        || remaining > total
    {
        return None;
    }
    let q = &pending.questions[index - 1];
    let title = (1..rows.len()).find(|&i| !rows[i].trim().is_empty())?;
    let first_option = (title + 1..rows.len()).find(|&i| numbered(rows[i]).is_some())?;
    if !rows_equal(&rows[title..first_option], &q.text) {
        return None;
    }
    let mut dialog = result(QuestionStep::CodexQuestion {
        index: index - 1,
        total,
    });
    let mut i = first_option;
    while i < rows.len() - 1 {
        let row = rows[i];
        if row.trim().is_empty() {
            i += 1;
            continue;
        }
        if let Some((number, label, selected)) = numbered(row) {
            if number as usize != dialog.options.len() + 1 {
                return None;
            }
            let end = option_end(rows, i, rows.len() - 1);
            let mut text_rows = vec![label];
            text_rows.extend_from_slice(&rows[i + 1..end]);
            let (label, description) = if let Some(option) = q.options.get(dialog.options.len()) {
                let expected = format!(
                    "{}{}",
                    option.label,
                    option.description.as_deref().unwrap_or("")
                );
                if !rows_equal(&text_rows, &expected) {
                    return None;
                }
                (option.label.clone(), option.description.clone())
            } else {
                if dialog.options.len() != q.options.len() || !q.allows_other {
                    return None;
                }
                let description = "Optionally, add details in notes (tab)";
                if !rows_equal(&text_rows, &format!("None of the above{description}")) {
                    return None;
                }
                ("None of the above".into(), Some(description.into()))
            };
            if selected {
                cursor(&mut dialog, QuestionCursor::Option(number))?;
            }
            dialog.options.push(ScreenOption {
                number,
                label,
                description,
                checked: None,
            });
            i = end;
        } else {
            let notes = notes_text(row)?;
            // Notes can be active while an option remains highlighted.
            if dialog
                .free_text
                .as_ref()
                .is_some_and(|f: &FreeText| f.active)
            {
                return None;
            }
            dialog.cursor = Some(QuestionCursor::Notes);
            dialog.free_text = Some(FreeText {
                active: true,
                text: notes.into(),
            });
            i += 1;
        }
    }
    if dialog.options.len() != q.options.len() + usize::from(q.allows_other) {
        return None;
    }
    if dialog.free_text.is_none() {
        dialog.free_text = Some(FreeText {
            active: false,
            text: String::new(),
        });
    }
    Some(dialog)
}

fn asynchronous(rows: &[&str], pending: &PendingQuestion) -> Option<QuestionScreen> {
    let body: Vec<_> = rows[1..]
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty())
        .collect();
    if let Some(notice) = body.first()?.strip_prefix("? ") {
        if body.len() != 2 || body[1] != "⇧← to answer" {
            return None;
        }
        let (count, countdown) = notice.split_once(" · ")?;
        let count = count
            .strip_suffix(" questions")
            .or_else(|| count.strip_suffix(" question"))?;
        if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let count: usize = count.parse().ok()?;
        if count == 0 || countdown.trim().is_empty() {
            return None;
        }
        let mut dialog = result(QuestionStep::AsyncQueued);
        dialog.countdown = Some(countdown.into());
        dialog.queued_count = Some(count);
        return Some(dialog);
    }
    let first_option = (0..body.len()).find(|&i| numbered(body[i]).is_some())?;
    let matches: Vec<_> = pending
        .questions
        .iter()
        .enumerate()
        .filter(|(_, q)| rows_equal(&body[..first_option], &q.text))
        .collect();
    let [(index, q)] = matches.as_slice() else {
        return None;
    };
    let mut dialog = result(QuestionStep::AsyncPicker(*index));
    let mut i = first_option;
    while i < body.len() - 1 {
        let (number, label, selected) = numbered(body[i])?;
        let end = option_end(&body, i, body.len() - 1);
        let mut text_rows = vec![label];
        text_rows.extend_from_slice(&body[i + 1..end]);
        let visible_label = text_rows
            .iter()
            .map(|r| r.trim())
            .filter(|r| !r.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let mut label = visible_label.as_str();
        if number as usize != dialog.options.len() + 1 {
            return None;
        }
        if let Some(option) = q.options.get(dialog.options.len()) {
            if !rows_equal(&text_rows, &option.label) {
                return None;
            }
            label = &option.label;
        } else {
            if dialog.options.len() != q.options.len() || !q.allows_other {
                return None;
            }
            dialog.free_text = Some(FreeText {
                active: selected,
                text: if label == "Other" {
                    String::new()
                } else {
                    label.into()
                },
            });
        }
        if selected {
            cursor(&mut dialog, QuestionCursor::Option(number))?;
        }
        dialog.options.push(ScreenOption {
            number,
            label: label.into(),
            description: None,
            checked: None,
        });
        i = end;
    }
    if dialog.options.len() != q.options.len() + usize::from(q.allows_other) {
        return None;
    }
    Some(dialog)
}

/// Indices in steps are zero-based; option numbers retain their terminal values.
pub fn parse(
    kind: QuestionKind,
    screen: &str,
    pending: &PendingQuestion,
) -> Option<QuestionScreen> {
    if kind != pending.kind || pending.questions.is_empty() {
        return None;
    }
    let rows: Vec<_> = screen.lines().collect();
    let mut chosen = None;
    for start in 0..rows.len() {
        let candidate = match kind {
            QuestionKind::ClaudeAsk => {
                let row = rows[start].trim();
                row.starts_with("← ") || row.starts_with("☐ ") || row.starts_with("☒ ")
            }
            QuestionKind::CodexPlan => rows[start].trim().starts_with("Question "),
            QuestionKind::CodexAsync => rows[start].trim() == "• Queued follow-up inputs",
        };
        if !candidate {
            continue;
        }
        let end = (start + 1..rows.len()).find(|&i| match kind {
            QuestionKind::ClaudeAsk => {
                rows[i].contains("Esc to cancel")
                    || rows[i].trim().trim_start_matches('❯').trim() == "2. Cancel"
            }
            QuestionKind::CodexPlan => {
                rows[i].contains("enter to submit answer")
                    || rows[i].contains("enter to submit all")
            }
            QuestionKind::CodexAsync => {
                rows[i].trim() == "⇧← to answer"
                    || rows[i].trim() == "enter submit   ⌃] skip   ⇧→ main prompt"
            }
        });
        let Some(end) = end else {
            continue;
        };
        let slice = &rows[start..=end];
        let dialog = match kind {
            QuestionKind::ClaudeAsk => claude(slice, pending),
            QuestionKind::CodexPlan => sync(slice, pending),
            QuestionKind::CodexAsync => asynchronous(slice, pending),
        };
        if let Some(mut dialog) = dialog {
            dialog.fingerprint = fingerprint(slice);
            if chosen.replace(dialog).is_some() {
                return None;
            }
        }
    }
    chosen
}

#[cfg(test)]
mod tests;
