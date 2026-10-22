//! Screen parsing anchored to structured pending questions. All dialog rows enter the fingerprint.
use crate::question_match::{PendingQuestion, Question, QuestionKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionStep {
    /// A one-question Claude single-select submits directly; multi-select uses review.
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
    pub unanswered: Option<usize>,
    pub completed_tabs: bool,
    /// Codex retains its highlighted option while the notes editor has focus.
    pub highlighted: Option<u32>,
    /// Whitespace-free review answers, stable across terminal hard wrapping.
    pub review_answers: Vec<String>,
    /// Lossless length-delimited rows, including whitespace, selection, and footer.
    pub fingerprint: Vec<u8>,
}

impl QuestionScreen {
    pub fn is_fresh(&self, pending: &PendingQuestion) -> bool {
        let untouched = self.options.iter().all(|o| o.checked != Some(true))
            && self.free_text.as_ref().is_none_or(|t| t.text.is_empty() && !t.active);
        untouched && match self.step {
            QuestionStep::ClaudeQuestion(0) => !self.completed_tabs
                && self.cursor == Some(QuestionCursor::Option(1)),
            QuestionStep::CodexQuestion { index: 0, total } => self.unanswered == Some(total),
            QuestionStep::AsyncQueued => true,
            QuestionStep::AsyncPicker(0) => self.cursor == Some(QuestionCursor::Option(1))
                && pending.questions.len() == 1,
            _ => false,
        }
    }
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

// Joined text is only a phone hint; slice B validates typed text with whitespace removed.
fn joined(rows: &[&str]) -> String {
    rows.iter()
        .map(|row| row.trim())
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn notes_text(row: &str) -> Option<&str> {
    let row = row.trim();
    if row == "›" {
        Some("")
    } else {
        row.strip_prefix("› ")
    }
}

fn indent(row: &str) -> usize {
    row.chars().take_while(|c| c.is_whitespace()).count()
}

// Consume known content by its structured value, even when it resembles a control.
fn known_end(rows: &[&str], start: usize, first: &str, expected: &str) -> Option<usize> {
    rows.get(start)?;
    let target = compact(expected);
    let mut seen = compact(first);
    let mut end = start + 1;
    loop {
        if seen == target {
            return Some(end);
        }
        if !target.starts_with(&seen) {
            return None;
        }
        seen.push_str(&compact(rows.get(end)?));
        end += 1;
    }
}

fn title_end(rows: &[&str], start: usize, expected: &str) -> Option<usize> {
    let mut end = known_end(rows, start, rows.get(start)?, expected)?;
    while rows.get(end).is_some_and(|r| r.trim().is_empty()) {
        end += 1;
    }
    Some(end)
}

// Unknown typed text can continue only deeper than the option-number column.
fn option_end(rows: &[&str], start: usize, limit: usize, multi: bool) -> usize {
    let Some(first) = rows.get(start) else {
        return limit.min(rows.len());
    };
    let column = indent(first)
        + if numbered(first).is_some_and(|(_, _, selected)| selected) {
            2
        } else {
            0
        };
    (start + 1..limit)
        .find(|&i| {
            rows.get(i).is_some_and(|row| {
                !row.trim().is_empty()
                    && (indent(row) <= column || (multi && indent(row) == 5 && submit(row)))
            })
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
        unanswered: None,
        completed_tabs: false,
        highlighted: None,
        review_answers: Vec::new(),
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

/// observed 2026-10-22: a single multi-select still uses tabs and review;
/// only a single single-select question submits directly from its bare picker.
pub(crate) fn claude_has_review(pending: &PendingQuestion) -> bool {
    pending.questions.len() > 1 || pending.questions.first().is_some_and(|q| q.multi_select)
}

fn tab_matches(row: &str, pending: &PendingQuestion) -> bool {
    if pending.questions.len() == 1 && !claude_has_review(pending) {
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
    if !tab_matches(rows.first()?, pending) {
        return None;
    }
    let title = rows
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, row)| !row.trim().is_empty())
        .map(|(index, _)| index)?;
    let mut matches = pending
        .questions
        .iter()
        .enumerate()
        .filter_map(|(index, q)| {
            let first_option = title_end(rows, title, &q.text)?;
            claude_question(rows, first_option, index, q)
        });
    let question = matches.next();
    if matches.next().is_some() {
        return None;
    }
    let reviewed = if claude_has_review(pending) {
        let review_end = title_end(rows, title, "Review your answers");
        review_end.and_then(|end| review(rows, end.saturating_sub(1), pending))
    } else {
        None
    };
    match (question, reviewed) {
        (Some(_), Some(_)) => None,
        (Some(mut dialog), None) | (None, Some(mut dialog)) => {
            dialog.completed_tabs = rows[0].contains('☒');
            Some(dialog)
        },
        (None, None) => None,
    }
}

fn claude_question(
    rows: &[&str],
    first_option: usize,
    index: usize,
    q: &Question,
) -> Option<QuestionScreen> {
    let mut dialog = result(QuestionStep::ClaudeQuestion(index));
    let mut divider = None;
    let mut submitted = false;
    let mut i = first_option;
    while i < rows.len() {
        if rule(rows.get(i)?) && indent(rows.get(i)?) == 0 {
            divider = Some(i);
            break;
        }
        if rows.get(i)?.trim().is_empty() {
            i += 1;
            continue;
        }
        if submit(rows.get(i)?) && q.multi_select {
            if indent(rows.get(i)?) != 5 && !rows.get(i)?.starts_with("❯    Submit") {
                return None;
            }
            if submitted || dialog.options.len() != q.options.len() + usize::from(q.allows_other) {
                return None;
            }
            submitted = true;
            if rows.get(i)?.contains('❯') {
                cursor(&mut dialog, QuestionCursor::Submit)?;
            }
            i += 1;
            continue;
        }
        let (number, mut label, selected) = numbered(rows.get(i)?)?;
        if indent(rows.get(i)?) != if selected { 0 } else { 2 } {
            return None;
        }
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
        let end;
        let mut description = None;
        if let Some(option) = q.options.get(dialog.options.len()) {
            if label != option.label {
                return None;
            }
            end = if let Some(expected) = &option.description {
                description = Some(expected.clone());
                known_end(rows, i + 1, rows.get(i + 1)?, expected)?
            } else {
                i + 1
            };
        } else {
            if dialog.options.len() != q.options.len() || !q.allows_other {
                return None;
            }
            end = option_end(rows, i, rows.len(), q.multi_select);
            let placeholder = if q.multi_select {
                "Type something"
            } else {
                "Type something."
            };
            let mut text_rows = vec![label];
            text_rows.extend_from_slice(rows.get(i + 1..end)?);
            let typed = !rows_equal(&text_rows, placeholder);
            dialog.free_text = Some(FreeText {
                active: selected || checked == Some(true),
                text: if typed {
                    joined(&text_rows)
                } else {
                    String::new()
                },
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
    let divider = divider?;
    let after: Vec<_> = rows
        .get(divider + 1..)?
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
    let body: Vec<_> = rows
        .get(title + 1..)?
        .iter()
        .copied()
        .filter(|r| !r.trim().is_empty())
        .collect();
    let mut i = 0;
    let mut answers = Vec::new();
    for q in &pending.questions {
        // Captured review fields: question marker at column 1, answer at column 3.
        if indent(body.get(i)?) != 1 {
            return None;
        }
        let first = body.get(i)?.trim().strip_prefix("● ")?;
        i = known_end(&body, i, first, &q.text)?;
        if indent(body.get(i)?) != 3 {
            return None;
        }
        let first = body.get(i)?.trim().strip_prefix('→')?.trim();
        let start = i + 1;
        i = start;
        while body.get(i).is_some_and(|row| indent(row) > 3) {
            i += 1;
        }
        let mut answer_rows = vec![first];
        answer_rows.extend_from_slice(body.get(start..i)?);
        answers.push(compact(&joined(&answer_rows)));
        if rows_equal(&answer_rows, "") {
            return None;
        }
    }
    if *body.get(i)? != "Ready to submit your answers?" || body.len() != i + 3 {
        return None;
    }
    let mut dialog = result(QuestionStep::ClaudeReview);
    dialog.review_answers = answers;
    for (offset, expected) in ["Submit answers", "Cancel"].iter().enumerate() {
        let (number, label, selected) = numbered(body.get(i + 1 + offset)?)?;
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
    let header = rows.first()?.trim().strip_prefix("Question ")?;
    let (position, unanswered) = header.split_once(" (")?;
    let remaining: usize = unanswered.strip_suffix(" unanswered)")?.parse().ok()?;
    let (index, total) = position.split_once('/')?;
    let index: usize = index.parse().ok()?;
    let total: usize = total.parse().ok()?;
    if total != pending.questions.len()
        || index == 0
        || index > total
        || remaining > total
    {
        return None;
    }
    let q = pending.questions.get(index.checked_sub(1)?)?;
    let title = (1..rows.len()).find(|&i| rows.get(i).is_some_and(|row| !row.trim().is_empty()))?;
    let first_option = title_end(rows, title, &q.text)?;
    let mut dialog = result(QuestionStep::CodexQuestion {
        index: index - 1,
        total,
    });
    dialog.unanswered = Some(remaining);
    let mut i = first_option;
    while i < rows.len().checked_sub(1)? {
        let row = rows.get(i)?;
        if row.trim().is_empty() {
            i += 1;
            continue;
        }
        if let Some((number, label, selected)) = numbered(row) {
            if indent(row) != if selected { 2 } else { 4 } {
                return None;
            }
            if number as usize != dialog.options.len() + 1 {
                return None;
            }
            let end;
            let (label, description) = if let Some(option) = q.options.get(dialog.options.len()) {
                let expected = format!(
                    "{}{}",
                    option.label,
                    option.description.as_deref().unwrap_or("")
                );
                end = known_end(rows, i, label, &expected)?;
                (option.label.clone(), option.description.clone())
            } else {
                if dialog.options.len() != q.options.len() || !q.allows_other {
                    return None;
                }
                let description = "Optionally, add details in notes (tab)";
                end = known_end(rows, i, label, &format!("None of the above{description}"))?;
                ("None of the above".into(), Some(description.into()))
            };
            if selected {
                dialog.highlighted = Some(number);
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
            if indent(row) != 2 {
                return None;
            }
            // Notes can be active while an option remains highlighted.
            if dialog
                .free_text
                .as_ref()
                .is_some_and(|f: &FreeText| f.active)
            {
                return None;
            }
            if rows
                .get(i + 1..rows.len().checked_sub(1)?)?
                .iter()
                .any(|r| !r.trim().is_empty() && indent(r) <= 4)
            {
                return None;
            }
            dialog.cursor = Some(QuestionCursor::Notes);
            dialog.free_text = Some(FreeText {
                active: true,
                text: {
                    let mut text_rows = vec![notes];
                    text_rows.extend_from_slice(rows.get(i + 1..rows.len().checked_sub(1)?)?);
                    let text = joined(&text_rows);
                    // observed 2026-10-22: Codex 0.163.0 shows Add notes while
                    // the active editor is empty. Literal identical input is
                    // indistinguishable; the following typed-text check fails safe.
                    if text == "Add notes" { String::new() } else { text }
                },
            });
            i = rows.len().checked_sub(1)?;
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
    let body: Vec<_> = rows
        .get(1..)?
        .iter()
        .copied()
        .filter(|r| !r.trim().is_empty())
        .collect();
    if let Some(notice) = body.first()?.trim().strip_prefix("? ") {
        if body.len() != 2 || body.get(1)?.trim() != "⇧← to answer" {
            return None;
        }
        let (count, countdown) = match notice.split_once(" · ") {
            Some((count, countdown)) if !countdown.trim().is_empty() => (count, Some(countdown)),
            Some(_) => return None,
            None => (notice, None),
        };
        let count = count
            .strip_suffix(" questions")
            .or_else(|| count.strip_suffix(" question"))?;
        if count.is_empty() || !count.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let count: usize = count.parse().ok()?;
        if count == 0 {
            return None;
        }
        let mut dialog = result(QuestionStep::AsyncQueued);
        dialog.countdown = countdown.map(str::to_owned);
        dialog.queued_count = Some(count);
        return Some(dialog);
    }
    let mut matches = pending
        .questions
        .iter()
        .enumerate()
        .filter_map(|(index, q)| {
            let first_option = title_end(&body, 0, &q.text)?;
            async_picker(&body, first_option, index, q)
        });
    let chosen = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(chosen)
    }
}

fn async_picker(
    body: &[&str],
    first_option: usize,
    index: usize,
    q: &Question,
) -> Option<QuestionScreen> {
    if body.len() < 2 || first_option >= body.len().checked_sub(1)? {
        return None;
    }
    let mut dialog = result(QuestionStep::AsyncPicker(index));
    let mut i = first_option;
    while i < body.len().checked_sub(1)? {
        let (number, label, selected) = numbered(body.get(i)?)?;
        if indent(body.get(i)?) != if selected { 2 } else { 4 } {
            return None;
        }
        let end = if let Some(option) = q.options.get(dialog.options.len()) {
            known_end(body, i, label, &option.label)?
        } else {
            option_end(body, i, body.len().checked_sub(1)?, false)
        };
        let mut text_rows = vec![label];
        text_rows.extend_from_slice(body.get(i + 1..end)?);
        let visible_label = joined(&text_rows);
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

fn compact(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Coalesce only recognized control blocks. Each logical row retains its original
/// span, so matching ignores visual wraps while fingerprints remain verbatim.
fn anchor_rows(screen: &str, pending: &PendingQuestion) -> (Vec<String>, Vec<(usize, usize)>) {
    let raw: Vec<_> = screen.lines().collect();
    let mut rows = Vec::new();
    let mut spans = Vec::new();
    let fixed = [
        ("Review your answers", 0),
        ("Ready to submit your answers?", 0),
        ("• Queued follow-up inputs", 0),
        ("⇧← to answer", 4),
        ("enter submit   ⌃] skip   ⇧→ main prompt", 2),
    ];
    let mut i = 0;
    while let Some(first_raw) = raw.get(i) {
        let start = i;
        let first = first_raw.trim();
        let mut replacement = None;
        for (expected, column) in fixed {
            if indent(first_raw) != column {
                continue;
            }
            let target = compact(expected);
            let mut candidate = String::new();
            for (offset, row) in raw.get(i..).unwrap_or_default().iter().enumerate() {
                if indent(row) > if column == 2 { 4 } else { column.max(2) } {
                    break;
                }
                candidate.push_str(&compact(row));
                if !target.starts_with(&candidate) {
                    break;
                }
                if candidate == target {
                    replacement = Some((i + offset + 1, expected.to_string()));
                    break;
                }
            }
            if replacement.is_some() {
                break;
            }
        }
        if replacement.is_none()
            && ((first.starts_with("← ") && indent(first_raw) == 0)
                || ((first.starts_with("☐ ") || first.starts_with("☒ ")) && indent(first_raw) <= 1))
        {
            let mut candidate = String::new();
            for (end, row) in raw.iter().enumerate().skip(i) {
                if indent(row)
                    > if indent(first_raw) == 2 {
                        4
                    } else {
                        indent(first_raw).max(2)
                    }
                {
                    break;
                }
                candidate.push_str(&compact(row));
                let mut rest = candidate.as_str();
                let tabbed = claude_has_review(pending);
                if tabbed {
                    rest = rest.strip_prefix('←').unwrap_or("");
                }
                let mut canonical = if tabbed {
                    "←  ".to_string()
                } else {
                    String::new()
                };
                let valid = pending.questions.iter().all(|q| {
                    let Some(state) = rest.chars().next().filter(|c| matches!(c, '☐' | '☒'))
                    else {
                        return false;
                    };
                    rest = rest.get(state.len_utf8()..).unwrap_or("");
                    let Some(header) = q.header.as_deref() else {
                        return false;
                    };
                    let key = compact(header);
                    let Some(after) = rest.strip_prefix(&key) else {
                        return false;
                    };
                    rest = after;
                    canonical.push_str(&format!("{state} {header}  "));
                    true
                });
                if valid && rest == if tabbed { "✔Submit→" } else { "" } {
                    if tabbed {
                        canonical.push_str("✔ Submit  →");
                    }
                    replacement = Some((end + 1, canonical));
                    break;
                }
                if row.trim().is_empty() {
                    break;
                }
            }
        }
        if replacement.is_none() && first.starts_with("Question ") && indent(first_raw) == 2 {
            let mut candidate = String::new();
            for (end, row) in raw.iter().enumerate().skip(i) {
                if indent(row)
                    > if indent(first_raw) == 2 {
                        4
                    } else {
                        indent(first_raw).max(2)
                    }
                {
                    break;
                }
                candidate.push_str(&compact(row));
                if let Some(body) = candidate
                    .strip_prefix("Question")
                    .and_then(|s| s.strip_suffix("unanswered)"))
                {
                    if let Some((position, remaining)) = body.split_once('(') {
                        replacement = Some((
                            end + 1,
                            format!("Question {position} ({remaining} unanswered)"),
                        ));
                    }
                    break;
                }
                if row.trim().is_empty() {
                    break;
                }
            }
        }
        if replacement.is_none() && first.starts_with("? ") && indent(first_raw) == 2 {
            let mut candidate = String::new();
            for (end, row) in raw.iter().enumerate().skip(i) {
                if indent(row) > 4 {
                    break;
                }
                candidate.push_str(&compact(row));
                // The answer shortcut is a distinct short marker after the banner.
                if candidate.ends_with("⇧←toanswer") {
                    let banner = candidate.trim_end_matches("⇧←toanswer");
                    if let Some(banner) = banner.strip_prefix('?') {
                        let (count, countdown) = match banner.split_once('·') {
                            Some((count, countdown)) => (count, Some(countdown)),
                            None => (banner, None),
                        };
                        let digits = count
                            .chars()
                            .take_while(char::is_ascii_digit)
                            .collect::<String>();
                        let suffix = count.get(digits.len()..).unwrap_or("");
                        if matches!(suffix, "question" | "questions") {
                            let countdown = countdown
                                .map(|text| format!(" · {text}"))
                                .unwrap_or_default();
                            rows.push(format!("? {digits} {suffix}{countdown}"));
                            spans.push((start, end + 1));
                            replacement = Some((end + 1, "⇧← to answer".into()));
                        }
                    }
                    break;
                }
                if row.trim().is_empty() {
                    break;
                }
            }
        }
        if replacement.is_none()
            && ((first.starts_with("Enter to select") && indent(first_raw) == 0)
                || (first.starts_with("tab ") && indent(first_raw) == 2))
        {
            let mut candidate = String::new();
            for (end, row) in raw.iter().enumerate().skip(i) {
                if indent(row)
                    > if indent(first_raw) == 2 {
                        4
                    } else {
                        indent(first_raw).max(2)
                    }
                {
                    break;
                }
                candidate.push_str(&compact(row));
                let done = if first.starts_with("Enter") {
                    candidate.ends_with("Esctocancel")
                } else if candidate.starts_with("taboresctoclearnotes") {
                    candidate.ends_with("entertosubmitanswer")
                        || candidate.ends_with("entertosubmitall")
                } else {
                    candidate.ends_with("esctointerrupt")
                };
                if done {
                    let canonical = if first.starts_with("Enter") {
                        "Enter to select · Esc to cancel"
                    } else if candidate.contains("entertosubmitanswer") {
                        "enter to submit answer"
                    } else {
                        "enter to submit all"
                    };
                    replacement = Some((end + 1, canonical.into()));
                    break;
                }
                if row.trim().is_empty() {
                    break;
                }
            }
        }
        if replacement.is_none() && rule(first_raw) && indent(first_raw) == 0 {
            let mut end = i + 1;
            while raw
                .get(end)
                .is_some_and(|row| rule(row) && indent(row) == 0)
            {
                end += 1;
            }
            replacement = Some((end, "─".into()));
        }
        if replacement.is_none() {
            if let Some((number, label, selected)) = numbered(first_raw)
                .filter(|(_, _, selected)| indent(first_raw) == if *selected { 0 } else { 2 })
            {
                let expected = "Chat about this";
                let mut candidate = compact(label);
                for (end, row) in raw.iter().enumerate().skip(i) {
                    if end > i {
                        if indent(row) > 2 {
                            break;
                        }
                        candidate.push_str(&compact(row));
                    }
                    if !compact(expected).starts_with(&candidate) {
                        break;
                    }
                    if candidate == compact(expected) {
                        replacement = Some((
                            end + 1,
                            format!("{}{number}. {expected}", if selected { "❯ " } else { "" }),
                        ));
                        break;
                    }
                }
            }
        }
        let (end, row) = replacement.unwrap_or((i + 1, first_raw.to_string()));
        rows.push(row);
        spans.push((start, end));
        i = end;
    }
    (rows, spans)
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
    let raw: Vec<_> = screen.lines().collect();
    let (logical, spans) = anchor_rows(screen, pending);
    let rows: Vec<_> = logical.iter().map(String::as_str).collect();
    let mut chosen = None;
    for start in 0..rows.len() {
        let candidate = match kind {
            QuestionKind::ClaudeAsk => {
                let row = rows[start].trim();
                (indent(raw[spans[start].0]) == 0 && row.starts_with("← "))
                    || (indent(raw[spans[start].0]) <= 1
                        && (row.starts_with("☐ ") || row.starts_with("☒ ")))
            }
            QuestionKind::CodexPlan => {
                indent(raw[spans[start].0]) == 2 && rows[start].trim().starts_with("Question ")
            }
            QuestionKind::CodexAsync => {
                indent(raw[spans[start].0]) == 0
                    && rows[start].trim() == "• Queued follow-up inputs"
            }
        };
        if !candidate {
            continue;
        }
        let ends = (start + 1..rows.len()).filter(|&i| {
            let Some(&(first, last)) = spans.get(i) else {
                return false;
            };
            let Some(source) = raw.get(first) else {
                return false;
            };
            let Some(row) = rows.get(i).map(|row| row.trim()) else {
                return false;
            };
            let column = indent(source);
            let original = source.trim();
            // End candidates are whole controls at their terminal columns, never
            // substrings inside a selected option, notes, or typed continuations.
            match kind {
                QuestionKind::ClaudeAsk => {
                    (column == 0
                        && original.starts_with("Enter ")
                        && row.starts_with("Enter to select")
                        && row.ends_with("Esc to cancel"))
                        || (column == 2 && original.starts_with("2. ") && row == "2. Cancel")
                        || (column == 0 && original.starts_with("❯ 2. ") && row == "❯ 2. Cancel")
                }
                QuestionKind::CodexPlan => {
                    column == 2
                        && original.starts_with("tab ")
                        && matches!(row, "enter to submit answer" | "enter to submit all")
                }
                QuestionKind::CodexAsync => {
                    (column == 2
                        && original.starts_with("enter ")
                        && row == "enter submit   ⌃] skip   ⇧→ main prompt")
                        || (row == "⇧← to answer"
                            && raw.get(first..last).is_some_and(|span| {
                                span.iter().any(|source| {
                                    indent(source) == 4 && source.trim().starts_with("⇧←")
                                })
                            }))
                }
            }
        });
        for end in ends {
            // A footer-looking content row cannot truncate the rest of the picker.
            let next = (spans[end].1..raw.len()).find(|&i| !raw[i].trim().is_empty());
            if next.is_some_and(|i| indent(raw[i]) > 4) {
                continue;
            }
            let slice = rows.get(start..=end)?;
            let dialog = match kind {
                QuestionKind::ClaudeAsk => claude(slice, pending),
                QuestionKind::CodexPlan => sync(slice, pending),
                QuestionKind::CodexAsync => asynchronous(slice, pending),
            };
            if let Some(mut dialog) = dialog {
                dialog.fingerprint = fingerprint(raw.get(spans.get(start)?.0..spans.get(end)?.1)?);
                if chosen.replace(dialog).is_some() {
                    return None;
                }
                break;
            }
        }
    }
    chosen
}

#[cfg(test)]
mod tests;
