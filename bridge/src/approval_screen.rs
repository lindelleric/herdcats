//! Fail-closed parsers for live approval dialogs. No hook data supplies labels.
use crate::hooks::Agent;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalDialog {
    pub agent: Agent,
    pub title: String,
    pub environment: Option<String>,
    pub detail: Vec<String>,
    pub reason_lines: Vec<String>,
    pub options: Vec<ApprovalOption>,
    pub selected: Option<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalOption {
    pub number: u32,
    pub label_lines: Vec<String>,
    pub key: OptionKey,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionKey {
    Digit(u32),
    Letter(char),
    Escape,
}
impl ApprovalDialog {
    /// Lossless length-delimited encoding of dialog content and selection keys.
    /// Selection is excluded so moving the highlight preserves dialog identity.
    pub fn canonical(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut field = |s: &str| {
            bytes.extend_from_slice(&(s.len() as u64).to_le_bytes());
            bytes.extend_from_slice(s.as_bytes());
        };
        field(self.agent.name());
        field(&self.title);
        field(if self.environment.is_some() {
            "some"
        } else {
            "none"
        });
        if let Some(environment) = &self.environment {
            field(environment);
        }
        for rows in [&self.detail, &self.reason_lines] {
            field(&rows.len().to_string());
            for row in rows {
                field(row);
            }
        }
        field(&self.options.len().to_string());
        for option in &self.options {
            match option.key {
                OptionKey::Digit(n) => {
                    field("digit");
                    field(&n.to_string());
                }
                OptionKey::Letter(c) => {
                    field("letter");
                    field(&c.to_string());
                }
                OptionKey::Escape => field("escape"),
            }
            field(&option.label_lines.len().to_string());
            for row in &option.label_lines {
                field(row);
            }
        }
        bytes
    }

    pub fn same_dialog(&self, other: &ApprovalDialog) -> bool {
        self.canonical() == other.canonical()
    }
}

pub fn parse(agent: Agent, screen: &str, command: &str) -> Option<ApprovalDialog> {
    match agent {
        Agent::Claude => parse_claude(screen, command),
        Agent::Codex => parse_codex(screen, command),
    }
}

fn rule(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && line.chars().all(|c| c == '╌')
}

fn leading_whitespace(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

fn command_row(line: &str, indent: &str) -> Option<String> {
    line.strip_prefix(indent)
        .map(|row| row.trim_end().to_string())
}

const KNOWN_CLAUDE_TITLES: &[&str] = &["Bash command"];
const KNOWN_CLAUDE_NOTICES: &[&str] = &["This command requires approval"];

/// Match exact row text, allowing only one visual-wrap separator per boundary.
fn rows_match_command(rows: &[String], command: &str) -> bool {
    fn search(
        rows: &[String],
        command: &str,
        row: usize,
        offset: usize,
        memo: &mut std::collections::HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(&matched) = memo.get(&(row, offset)) {
            return matched;
        }
        let matched = if !command[offset..].starts_with(&rows[row]) {
            false
        } else {
            let end = offset + rows[row].len();
            if row + 1 == rows.len() {
                command[end..].chars().all(char::is_whitespace)
            } else {
                // A blank screen row consumes real newline boundaries on both
                // sides; it cannot be invented by a zero-width or word wrap.
                let blank = rows[row].is_empty() || rows[row + 1].is_empty();
                (!blank && search(rows, command, row + 1, end, memo))
                    || ((!blank && command[end..].starts_with(' ')
                        || command[end..].starts_with('\n'))
                        && search(rows, command, row + 1, end + 1, memo))
            }
        };
        memo.insert((row, offset), matched);
        matched
    }
    !rows.is_empty() && search(rows, command, 0, 0, &mut std::collections::HashMap::new())
}

fn screen_rows(screen: &str) -> Vec<&str> {
    let mut rows: Vec<_> = screen.lines().rev().take(400).collect();
    rows.reverse();
    rows
}

fn detail_rows(lines: &[&str], indent: &str) -> Option<Vec<String>> {
    lines
        .iter()
        .map(|line| {
            if line.trim().is_empty() {
                Some(String::new())
            } else {
                command_row(line, indent)
            }
        })
        .collect()
}

pub fn parse_claude(screen: &str, command: &str) -> Option<ApprovalDialog> {
    let lines = screen_rows(screen);
    let mut chosen = None;
    for q in 0..lines.len() {
        if lines[q].trim() != "Do you want to proceed?" {
            continue;
        }
        let Some(footer) = (q + 1..lines.len()).find(|&i| {
            leading_whitespace(lines[i]) == leading_whitespace(lines[q])
                && lines[i].trim().starts_with("Esc to cancel")
                && lines[i - 1].trim().is_empty()
        }) else {
            continue;
        };
        let Some(r2) = (0..q).rev().find(|&i| rule(lines[i])) else {
            continue;
        };
        let reason_lines: Vec<_> = lines[r2 + 1..q].iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        if reason_lines.iter().any(|line| !KNOWN_CLAUDE_NOTICES.contains(&line.as_str())) {
            continue;
        }
        for r1 in 0..r2 {
            if !rule(lines[r1]) {
                continue;
            }
            let Some(detail) = detail_rows(&lines[r1 + 1..r2], leading_whitespace(lines[q])) else {
                continue;
            };
            if !rows_match_command(&detail, command) {
                continue;
            }
            let Some((options, selected)) = options(&lines[q + 1..footer], Agent::Claude) else {
                continue;
            };
            for title in 0..r1 {
                if !KNOWN_CLAUDE_TITLES.contains(&lines[title].trim()) {
                    continue;
                }
                // A rule already following this title starts its body;
                // a rule printed inside that body cannot start it again.
                if lines[title + 1..r1].iter().any(|l| rule(l)) {
                    continue;
                }
                let dialog = ApprovalDialog {
                    agent: Agent::Claude,
                    title: lines[title].trim().into(),
                    environment: None,
                    detail: detail.clone(),
                    reason_lines: reason_lines.clone(),
                    options: options.clone(),
                    selected,
                };
                if chosen.replace(dialog).is_some() {
                    return None;
                }
            }
        }
    }
    chosen
}

fn codex_metadata(lines: &[&str]) -> Option<(Option<String>, Vec<String>)> {
    let mut environment = None;
    let mut reason = Vec::new();
    for line in lines {
        let row = line.trim();
        if row.is_empty() {
            continue;
        }
        if let Some(value) = row.strip_prefix("Environment: ") {
            if value.trim().is_empty() || environment.is_some() {
                return None;
            }
            environment = Some(value.trim().to_string());
        } else if let Some(text) = row.strip_prefix("Reason: ") {
            if text.trim().is_empty() || !reason.is_empty() {
                return None;
            }
            reason.push(text.trim().to_string());
        } else if row.starts_with('$')
            || row.starts_with("Environment:")
            || row.starts_with("Reason:")
            || reason.is_empty()
        {
            return None;
        } else {
            reason.push(row.to_string());
        }
    }
    Some((environment, reason))
}

pub fn parse_codex(screen: &str, command: &str) -> Option<ApprovalDialog> {
    let lines = screen_rows(screen);
    let title = "Would you like to run the following command?";
    let mut chosen = None;
    let mut body_spans = Vec::new();
    for t in 0..lines.len() {
        if lines[t].trim() != title {
            continue;
        }
        // A title printed in a previously recognized command body is body text.
        if body_spans.iter().any(|&(start, end)| start < t && t < end) {
            continue;
        }
        for s in t + 1..lines.len() {
            let Some(first_row) = lines[s].trim_start().strip_prefix("$ ") else {
                continue;
            };
            if first_row.trim().is_empty() {
                continue;
            }
            let Some((environment, reason_lines)) = codex_metadata(&lines[t + 1..s]) else {
                continue;
            };
            let Some(first_option) =
                (s + 1..lines.len()).find(|&i| option_line(lines[i], '›').is_some())
            else {
                continue;
            };
            let Some(mut detail) =
                detail_rows(&lines[s + 1..first_option], leading_whitespace(lines[s]))
            else {
                continue;
            };
            // Blank rows after the command are dialog separators. Preserve
            // internal blank rows so the hook command must still match exactly.
            while detail.last().is_some_and(String::is_empty) {
                detail.pop();
            }
            detail.insert(0, first_row.trim_end().to_string());
            let Some(footer) = (first_option + 1..lines.len()).find(|&i| {
                leading_whitespace(lines[i]) == leading_whitespace(lines[t])
                    && lines[i].trim() == "Press enter to confirm or esc to cancel"
            }) else {
                continue;
            };
            if !rows_match_command(&detail, command) {
                continue;
            }
            let Some((options, selected)) = options(&lines[first_option..footer], Agent::Codex)
            else {
                continue;
            };
            body_spans.push((s, first_option));
            let dialog = ApprovalDialog {
                agent: Agent::Codex,
                title: title.into(),
                environment,
                detail,
                reason_lines,
                options,
                selected,
            };
            if chosen.replace(dialog).is_some() {
                return None;
            }
        }
    }
    chosen
}

fn option_line(line: &str, marker: char) -> Option<(u32, &str, bool, usize)> {
    let trimmed = line.trim_start();
    let selected = trimmed.starts_with(marker);
    let body = if selected {
        trimmed.strip_prefix(marker)?.strip_prefix(' ')?
    } else {
        trimmed
    };
    let (number, label) = body.split_once(". ")?;
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let indent = line.len() - body.len();
    Some((number.parse().ok()?, label.trim(), selected, indent))
}

fn options(lines: &[&str], agent: Agent) -> Option<(Vec<ApprovalOption>, Option<usize>)> {
    let marker = if agent == Agent::Claude { '❯' } else { '›' };
    let mut options: Vec<ApprovalOption> = Vec::new();
    let mut selected = None;
    let mut indent = 0;
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        if let Some((number, label, highlighted, column)) = option_line(line, marker) {
            if number != u32::try_from(options.len()).ok()?.checked_add(1)? || label.is_empty() {
                return None;
            }
            if highlighted {
                if selected.is_some() {
                    return None;
                }
                selected = Some(options.len());
            }
            indent = column;
            options.push(ApprovalOption {
                number,
                label_lines: vec![label.into()],
                key: OptionKey::Digit(number),
            });
        } else {
            let column = line.len() - line.trim_start().len();
            if column <= indent {
                return None;
            }
            let last = options.last_mut()?;
            last.label_lines.push(line.trim().to_string());
        }
    }
    if options.is_empty() {
        return None;
    }
    if agent == Agent::Codex {
        for option in &mut options {
            let last = option.label_lines.last_mut()?;
            let (label, shortcut) = if let Some(shortcut) = last.strip_prefix('(') {
                ("", shortcut)
            } else {
                last.rsplit_once(" (")?
            };
            let shortcut = shortcut.strip_suffix(')')?;
            option.key = if shortcut == "esc" {
                OptionKey::Escape
            } else {
                let mut chars = shortcut.chars();
                let c = chars.next()?;
                if !c.is_ascii_alphabetic() || chars.next().is_some() {
                    return None;
                }
                OptionKey::Letter(c)
            };
            *last = label.trim_end().to_string();
            if last.is_empty() {
                option.label_lines.pop();
            }
            if option.label_lines.is_empty() {
                return None;
            }
        }
    }
    Some((options, selected))
}
