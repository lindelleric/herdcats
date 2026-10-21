//! Fail-closed parsers for live approval dialogs. No hook data supplies labels.
use crate::hooks::Agent;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalDialog {
    pub agent: Agent,
    pub title: String,
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
    /// FNV-1a over length-delimited UTF-8 fields; independent of selection and
    /// Rust's unspecified default hasher. Field boundaries cannot alias.
    pub fn fingerprint(&self) -> String {
        let mut hash = 0xcbf29ce484222325u64;
        let mut field = |s: &str| {
            for byte in (s.len() as u64).to_le_bytes().iter().chain(s.as_bytes()) {
                hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3);
            }
        };
        field(&self.title);
        for rows in [&self.detail, &self.reason_lines] {
            field(&rows.len().to_string());
            for row in rows {
                field(row);
            }
        }
        field(&self.options.len().to_string());
        for option in &self.options {
            field(&option.label_lines.len().to_string());
            for row in &option.label_lines {
                field(row);
            }
        }
        format!("{hash:016x}")
    }
}

pub fn parse(agent: Agent, screen: &str) -> Option<ApprovalDialog> {
    match agent {
        Agent::Claude => parse_claude(screen),
        Agent::Codex => parse_codex(screen),
    }
}

fn rule(line: &str) -> bool {
    let line = line.trim();
    !line.is_empty() && line.chars().all(|c| c == '╌')
}

const KNOWN_CLAUDE_TITLES: &[&str] = &["Bash command"];

pub fn parse_claude(screen: &str) -> Option<ApprovalDialog> {
    let lines: Vec<_> = screen.lines().collect();
    let question = lines
        .iter()
        .rposition(|l| l.trim() == "Do you want to proceed?")?;
    let end_rule = (0..question).rev().find(|&i| rule(lines[i]))?;
    let start_rule = (0..end_rule).rev().find(|&i| rule(lines[i]))?;
    let title_row = (0..start_rule)
        .rev()
        .find(|&i| KNOWN_CLAUDE_TITLES.contains(&lines[i].trim()))?;
    let title = lines[title_row].trim().to_string();
    // Tip and description rows cannot be distinguished reliably when wrapped.
    let footer =
        (question + 1..lines.len()).find(|&i| lines[i].trim().starts_with("Esc to cancel"))?;
    let (options, selected) = options(&lines[question + 1..footer], Agent::Claude)?;
    Some(ApprovalDialog {
        agent: Agent::Claude,
        title,
        detail: lines[start_rule + 1..end_rule]
            .iter()
            .map(|l| l.trim().to_string())
            .collect(),
        reason_lines: Vec::new(),
        options,
        selected,
    })
}

pub fn parse_codex(screen: &str) -> Option<ApprovalDialog> {
    let lines: Vec<_> = screen.lines().collect();
    let title = "Would you like to run the following command?";
    let start = lines.iter().rposition(|l| l.trim() == title)?;
    let footer = (start + 1..lines.len())
        .find(|&i| lines[i].trim() == "Press enter to confirm or esc to cancel")?;
    let first = (start + 1..footer).find(|&i| option_line(lines[i], '›').is_some())?;
    let mut detail = Vec::new();
    let mut reason_lines = Vec::new();
    // A body row belongs to the most recently introduced reason or command.
    // Environment metadata does not contribute body rows.
    let mut context = None;
    for line in &lines[start + 1..first] {
        let row = line.trim();
        if row.is_empty() {
            continue;
        }
        if let Some(command) = row.strip_prefix("$ ") {
            if !detail.is_empty() || command.trim().is_empty() {
                return None;
            }
            detail.push(command.trim().to_string());
            context = Some(false);
        } else if let Some(text) = row.strip_prefix("Reason: ") {
            if !reason_lines.is_empty() || text.trim().is_empty() {
                return None;
            }
            reason_lines.push(text.trim().to_string());
            context = Some(true);
        } else if let Some(environment) = row.strip_prefix("Environment: ") {
            if environment.trim().is_empty() {
                return None;
            }
        } else if row == "$" || row.starts_with("Reason:") || row.starts_with("Environment:") {
            return None;
        } else {
            match context {
                Some(true) => reason_lines.push(row.to_string()),
                Some(false) => detail.push(row.to_string()),
                None => return None,
            }
        }
    }
    if detail.is_empty() {
        return None;
    }
    let (options, selected) = options(&lines[first..footer], Agent::Codex)?;
    Some(ApprovalDialog {
        agent: Agent::Codex,
        title: title.into(),
        detail,
        reason_lines,
        options,
        selected,
    })
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
