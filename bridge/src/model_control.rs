//! Pure model/effort readers and guarded picker plans for model control.
//!
//! Reading follows the sources verified in `docs/model-effort-picker.md`: the
//! Claude transcript's assistant records (with settings.json fallbacks) and the
//! Codex rollout's `turn_context` records (with config.toml fallbacks). Screen
//! parsing works on Herdr's plain-text visible pane output, mirroring
//! `approval_screen`/`question_screen`. Nothing here performs I/O.

use crate::Result;
use serde_json::Value;

/// Tier aliases Claude Code accepts for `/model`, with their static labels.
/// Updated only when the bridge ships; unknown transcript models still display
/// via `claude_model_label`, so staleness is cosmetic.
pub const CLAUDE_MODELS: &[(&str, &str)] = &[
    ("fable", "Fable"),
    ("opus", "Opus"),
    ("sonnet", "Sonnet"),
    ("haiku", "Haiku"),
];

/// What Claude Code runs when settings.json names no model; `/model default`
/// selects it again.
pub const CLAUDE_DEFAULT_MODEL: &str = "default";

/// Current version behind each tier alias, so lists show "Opus 5.5" rather
/// than a bare tier. Update with each Claude release; the live transcript id
/// still wins for the model in use.
const CLAUDE_ALIAS_VERSIONS: &[(&str, &str)] = &[
    ("fable", "5.1"),
    ("opus", "5.5"),
    ("sonnet", "5.5"),
    ("haiku", "5.5"),
];

/// Full id a tier alias resolves to ("sonnet" → "claude-sonnet-5-5").
pub fn claude_alias_id(alias: &str) -> Option<String> {
    CLAUDE_ALIAS_VERSIONS
        .iter()
        .find(|(name, _)| *name == alias)
        .map(|(name, version)| format!("claude-{name}-{}", version.replace('.', "-")))
}

/// Whether a confirmed label ("Haiku 5.5") names the requested alias or id,
/// matched by its derived label as a case-insensitive prefix.
pub fn claude_label_names(label: &str, wanted: &str) -> bool {
    let expected = claude_model_label(wanted).to_ascii_lowercase();
    !wanted.is_empty() && label.trim().to_ascii_lowercase().starts_with(&expected)
}

/// Label shown to people: a versioned name for tier aliases ("Opus 5.5") and
/// the derived name for concrete ids. Dialog matching keeps using the bare
/// tier from `claude_model_label`, so a newer CLI version still matches.
pub fn claude_display_label(id: &str) -> String {
    match CLAUDE_ALIAS_VERSIONS.iter().find(|(alias, _)| *alias == id) {
        Some((_, version)) => format!("{} {version}", claude_model_label(id)),
        None => claude_model_label(id),
    }
}

/// Effort levels offered in the UI. `auto` is valid for the CLI but hidden.
pub const CLAUDE_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];

/// Effort ids the Codex picker can show; `minimal`/`persistent` exist in the
/// CLI enum but are not offered by the picker.
pub const CODEX_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];

/// Label shown for an effort id in both agents' lists.
pub fn effort_label(effort: &str) -> &str {
    match effort {
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "Extra high",
        "max" => "Max",
        "ultra" => "Ultra",
        "minimal" => "Minimal",
        other => other,
    }
}

/// True when the effort id is one this bridge may send to an agent.
pub fn known_effort(agent: &str, effort: &str) -> bool {
    match agent {
        "claude" => effort == "auto" || CLAUDE_EFFORTS.contains(&effort),
        // `minimal` has no picker row, so it is neither listed nor sendable.
        "codex" => CODEX_EFFORTS.contains(&effort),
        _ => false,
    }
}

/// "claude-opus-5-5" → "Opus 5.5", "opus" → "Opus", unknown ids pass through.
/// Dated suffixes ("20251001") and variant markers ("[1m]") are dropped.
pub fn claude_model_label(id: &str) -> String {
    if let Some((_, label)) = CLAUDE_MODELS.iter().find(|(alias, _)| *alias == id) {
        return (*label).into();
    }
    if id == CLAUDE_DEFAULT_MODEL {
        return "Default".into();
    }
    let body = match id.strip_prefix("claude-") {
        Some(body) => body,
        // Ids outside the claude- family (gateway aliases, ARNs) pass through
        // untouched rather than being half-prettified.
        None => return id.into(),
    };
    let mut parts = body.split('-');
    let Some(name) = parts.next() else {
        return id.into();
    };
    let mut label = String::with_capacity(id.len());
    let mut chars = name.chars();
    if let Some(first) = chars.next() {
        label.extend(first.to_uppercase());
        label.push_str(chars.as_str());
    }
    let mut version = false;
    for part in parts {
        // Version segments are one or two digits ("5", "41"); stop at dates,
        // variant markers, or any other non-version tail. "opus-5-5" keeps a
        // space before the version and dots between its segments.
        if !part.chars().all(|c| c.is_ascii_digit()) || part.len() > 2 {
            break;
        }
        label.push(if version { '.' } else { ' ' });
        label.push_str(part);
        version = true;
    }
    if label.is_empty() { id.into() } else { label }
}

/// "gpt-6-luna" → "GPT-6-Luna" for slugs missing from the catalog.
pub fn codex_slug_label(slug: &str) -> String {
    slug.split('-')
        .map(|part| match part {
            "gpt" => "GPT".to_owned(),
            digits if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) => {
                digits.to_owned()
            }
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join("-")
}

/// Last assistant `message.model` plus the effort in effect, from a tail of
/// Claude transcript records. Effort chain: the latest assistant record's
/// `effort` field, else the latest `api-request`'s
/// `params.output_config.effort` (older transcripts omit the former).
pub fn claude_scan(records: &[Value]) -> Option<(String, Option<String>)> {
    let mut model = None;
    let mut assistant_effort = None;
    let mut request_effort = None;
    for record in records {
        // Subagent (sidechain) and meta records carry their own model and
        // effort; like the timeline parser, only main-session records count.
        if record["isSidechain"] == true || record["isMeta"] == true {
            continue;
        }
        match record["type"].as_str() {
            // A desk-side `/model` or `/effort` is recorded only as its local
            // command output until the next reply; it changes the pair now.
            Some("user") => match claude_local_command_change(&record["message"]["content"]) {
                // Claude keeps the session's effort across a model switch.
                Some(LocalChange::Model(id)) => model = Some(id),
                Some(LocalChange::Effort(level)) => {
                    assistant_effort = Some(level);
                    request_effort = None;
                }
                None => {}
            },
            Some("assistant") => {
                if let Some(id) = record["message"]["model"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                {
                    model = Some(id.to_owned());
                    // Each assistant record replaces the previous one's effort,
                    // so a record without the field defers to the requests.
                    assistant_effort = record["effort"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned);
                }
            }
            Some("api-request") => {
                if let Some(level) = record["params"]["output_config"]["effort"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                {
                    request_effort = Some(level.to_owned());
                }
            }
            _ => {}
        }
    }
    model.map(|id| (id, assistant_effort.or(request_effort)))
}

enum LocalChange {
    Model(String),
    Effort(String),
}

/// Parses a `<local-command-stdout>` user record from a confirmed `/model` or
/// `/effort`: "Set model to Haiku 5.5 …" → `haiku`, "Set effort level to low
/// (…" → `low`. "Kept …" (a declined switch) and other output change nothing.
fn claude_local_command_change(content: &Value) -> Option<LocalChange> {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let body = text
        .trim()
        .strip_prefix("<local-command-stdout>")?
        .split("</local-command-stdout>")
        .next()?
        .trim();
    if let Some(rest) = body.strip_prefix("Set model to ") {
        let mut words = rest.split_whitespace();
        let tier = words.next()?.to_ascii_lowercase();
        if tier == CLAUDE_DEFAULT_MODEL {
            return Some(LocalChange::Model(tier));
        }
        let (alias, _) = CLAUDE_MODELS.iter().find(|(alias, _)| *alias == tier)?;
        // Keep the confirmed version: "Haiku 4.5" is claude-haiku-4-5, not
        // whatever `haiku` resolves to today.
        let version = words.next().filter(|word| {
            !word.is_empty() && word.split('.').all(|part| !part.is_empty() && part.len() <= 2 && part.chars().all(|c| c.is_ascii_digit()))
        });
        return Some(LocalChange::Model(match version {
            Some(version) => format!("claude-{alias}-{}", version.replace('.', "-")),
            None => (*alias).to_owned(),
        }));
    }
    let level = body.strip_prefix("Set effort level to ")?.split_whitespace().next()?;
    known_effort("claude", level).then(|| LocalChange::Effort(level.to_owned()))
}

/// The subset of `~/.claude/settings.json` model control needs. `model` is the
/// persisted default alias; `effort_level` a session-wide override;
/// `model_settings` maps full model ids to per-model `effortLevel` overrides.
pub struct ClaudeSettings {
    pub model: Option<String>,
    pub effort_level: Option<String>,
    pub model_settings: Vec<(String, String)>,
    /// `modelAliases`: alias → configured full id. Used only to find a
    /// per-model effort; it is not trusted for labels (it can be stale).
    pub model_aliases: Vec<(String, String)>,
}

pub fn parse_claude_settings(text: &str) -> ClaudeSettings {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => {
            return ClaudeSettings {
                model: None,
                effort_level: None,
                model_settings: Vec::new(),
                model_aliases: Vec::new(),
            };
        }
    };
    let string = |path: &str| -> Option<String> {
        value
            .pointer(path)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let model_settings = value
        .pointer("/modelSettings")
        .and_then(Value::as_object)
        .map(|models| {
            models
                .iter()
                .filter_map(|(id, entry)| {
                    entry
                        .pointer("/effortLevel")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(|effort| (id.clone(), effort.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    let model_aliases = value
        .pointer("/modelAliases")
        .and_then(Value::as_object)
        .map(|aliases| {
            aliases
                .iter()
                .filter_map(|(alias, id)| {
                    id.as_str()
                        .filter(|s| !s.is_empty())
                        .map(|id| (alias.clone(), id.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();
    ClaudeSettings {
        model: string("/model"),
        effort_level: string("/effortLevel"),
        model_settings,
        model_aliases,
    }
}

impl ClaudeSettings {
    /// `model` may be a tier alias (`sonnet`): `modelSettings` is keyed by full
    /// ids, so an alias also matches the id it currently resolves to.
    pub fn effort_for(&self, model: &str) -> Option<&str> {
        // Candidates in order: the id as given, the alias's configured id
        // (`modelAliases`), then the id this bridge resolves it to.
        let configured = self
            .model_aliases
            .iter()
            .find(|(alias, _)| alias == model)
            .map(|(_, id)| id.clone());
        [Some(model.to_owned()), configured, claude_alias_id(model)]
            .into_iter()
            .flatten()
            .find_map(|candidate| {
                self.model_settings
                    .iter()
                    .find(|(id, _)| *id == candidate)
                    .map(|(_, effort)| effort.as_str())
            })
            .or(self.effort_level.as_deref())
    }
}

/// Last `turn_context` `model`/`effort` pair from a tail of Codex rollout
/// records. Records from other sessions or malformed lines are ignored by the
/// caller's tail reader; here every `turn_context` with a model counts.
pub fn codex_scan(records: &[Value]) -> Option<(String, Option<String>)> {
    records
        .iter()
        .rev()
        .filter(|record| record["type"] == "turn_context")
        .find_map(|record| {
            record["payload"]["model"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|model| {
                    (
                        model.to_owned(),
                        record["payload"]["effort"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned),
                    )
                })
        })
}

/// Top-level `model` and `model_reasoning_effort` from Codex's config.toml.
pub fn parse_codex_config(text: &str) -> (Option<String>, Option<String>) {
    let document = match text.parse::<toml_edit::DocumentMut>() {
        Ok(document) => document,
        Err(_) => return (None, None),
    };
    let string = |key: &str| {
        document
            .get(key)
            .and_then(toml_edit::Item::as_value)
            .and_then(toml_edit::Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    (string("model"), string("model_reasoning_effort"))
}

/// One Codex catalog entry; only `visibility == "list"` rows are offered.
pub struct CodexCatalogModel {
    pub slug: String,
    pub label: String,
    pub efforts: Vec<String>,
}

/// Parses `~/.codex/models_cache.json`, filtered and ordered like the CLI's
/// picker: `visibility == "list"`, ascending `priority` (stable on ties).
pub fn parse_codex_catalog(text: &str) -> Vec<CodexCatalogModel> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let mut models: Vec<(i64, usize, CodexCatalogModel)> = value["models"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .enumerate()
                .filter_map(|(index, row)| {
                    if row["visibility"].as_str() != Some("list") {
                        return None;
                    }
                    let slug = row["slug"].as_str()?.to_owned();
                    if slug.is_empty() {
                        return None;
                    }
                    let label = row["display_name"]
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| slug.clone());
                    let efforts = row["supported_reasoning_levels"]
                        .as_array()
                        .map(|levels| {
                            levels
                                .iter()
                                .filter_map(|level| {
                                    level["effort"]
                                        .as_str()
                                        .filter(|s| !s.is_empty() && known_effort("codex", s))
                                        .map(str::to_owned)
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let priority = row["priority"].as_i64().unwrap_or(i64::MAX);
                    Some((
                        priority,
                        index,
                        CodexCatalogModel {
                            slug,
                            label,
                            efforts,
                        },
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    models.sort_by_key(|(priority, index, _)| (*priority, *index));
    models.into_iter().map(|(_, _, model)| model).collect()
}

/// The live idle footer is authoritative for session-only changes after reconnect.
/// Match an entire catalog display name and known effort before the Context delimiter.
pub fn codex_idle_footer(
    screen: &str,
    catalog: &[CodexCatalogModel],
) -> Option<(String, Option<String>)> {
    for line in screen.lines().rev() {
        let Some((prefix, _)) = line.trim().split_once(" · Context ") else {
            continue;
        };
        let mut matches = catalog.iter().filter_map(|model| {
            let effort = prefix
                .strip_prefix(model.label.as_str())?
                .strip_prefix(' ')?;
            let id = ["low", "medium", "high", "xhigh", "max", "ultra", "minimal"]
                .into_iter()
                .find(|id| {
                    effort.eq_ignore_ascii_case(id) || effort.eq_ignore_ascii_case(effort_label(id))
                })?;
            Some((model.slug.clone(), Some(id.to_owned())))
        });
        if let Some(pair) = matches.next()
            && matches.next().is_none()
        {
            return Some(pair);
        }
    }
    None
}

/// One numbered row of a picker screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerRow {
    pub number: u32,
    pub label: String,
    pub selected: bool,
}

/// A parsed Codex picker screen. `Composer` means no picker overlay is open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerScreen {
    ModelList(Vec<PickerRow>),
    EffortList {
        model_label: String,
        rows: Vec<PickerRow>,
    },
    Advanced(Vec<PickerRow>),
    Composer,
}

/// Codex decorates any picker row with these status suffixes in either order.
/// Preserve the displayed label in parsed rows, but compare only its base text.
pub fn picker_label(mut label: &str) -> &str {
    label = label.trim();
    loop {
        match label
            .strip_suffix(" (default)")
            .or_else(|| label.strip_suffix(" (current)"))
        {
            Some(base) => label = base.trim_end(),
            None => return label,
        }
    }
}

fn parse_rows(lines: &[&str]) -> Vec<PickerRow> {
    let mut rows = Vec::new();
    for line in lines {
        let trimmed = line.trim_start();
        let selected = trimmed.starts_with('›');
        let rest = trimmed.trim_start_matches(['›', ' ']);
        let Some((number, label)) = rest.split_once(". ") else {
            continue;
        };
        if number.is_empty() || !number.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        // Picker rows are "N. Label  Description"; the label ends at the first
        // run of two or more spaces (descriptions are aligned in columns).
        let label = label.split("  ").next().unwrap_or(label).trim_end();
        if label.is_empty() {
            continue;
        }
        rows.push(PickerRow {
            number: number.parse().unwrap_or(0),
            label: label.to_owned(),
            selected,
        });
    }
    rows
}

/// Parses the visible pane text into a picker state. Plain text only; Herdr's
/// `pane.read` strips terminal control sequences.
pub fn parse_codex_screen(screen: &str) -> PickerScreen {
    let lines: Vec<&str> = screen.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let line = line.trim();
        if line.contains("Select Model and Effort") {
            return PickerScreen::ModelList(parse_rows(&lines[index + 1..]));
        }
        if let Some(label) = line.strip_prefix("Select Reasoning Level for ") {
            return PickerScreen::EffortList {
                model_label: label.trim().to_owned(),
                rows: parse_rows(&lines[index + 1..]),
            };
        }
        if line.contains("Advanced Reasoning") {
            return PickerScreen::Advanced(parse_rows(&lines[index + 1..]));
        }
    }
    PickerScreen::Composer
}

impl PickerScreen {
    pub fn is_composer(&self) -> bool {
        matches!(self, PickerScreen::Composer)
    }
    /// The model list shows "(current)" on the active model's row.
    pub fn current_model_row(&self) -> Option<&PickerRow> {
        match self {
            PickerScreen::ModelList(rows) => rows.iter().find(|row| {
                row.label
                    .split_whitespace()
                    .rev()
                    .take_while(|part| matches!(*part, "(current)" | "(default)"))
                    .any(|part| part == "(current)")
            }),
            _ => None,
        }
    }
    pub fn rows(&self) -> &[PickerRow] {
        match self {
            PickerScreen::ModelList(rows) | PickerScreen::Advanced(rows) => rows,
            PickerScreen::EffortList { rows, .. } => rows,
            PickerScreen::Composer => &[],
        }
    }
    /// The parsed cursor (›) index, rejecting renders that mark several rows.
    pub fn cursor(&self) -> Option<usize> {
        let mut rows = self.rows().iter().enumerate();
        let first = rows.find(|(_, row)| row.selected).map(|(index, _)| index);
        if rows.any(|(_, row)| row.selected) {
            return None;
        }
        first
    }
    /// Arrow presses (as Herdr key tokens) moving the cursor from its parsed
    /// position to `target`, without relying on list wrap.
    pub fn arrow_plan(&self, target: usize) -> Option<Vec<&'static str>> {
        let rows = self.rows();
        if target >= rows.len() {
            return None;
        }
        let cursor = self.cursor()?;
        let plan = if target > cursor {
            vec!["down"; target - cursor]
        } else {
            vec!["up"; cursor - target]
        };
        Some(plan)
    }
    pub fn row_for_label(&self, label: &str) -> Option<usize> {
        let normalize = |text: &str| picker_label(text).to_ascii_lowercase();
        let wanted = normalize(label);
        self.rows()
            .iter()
            .position(|row| normalize(&row.label) == wanted)
    }
}

/// Row label the Codex effort picker shows for an effort id, including the
/// nested "More reasoning…" entry that hides max and ultra.
pub fn codex_effort_row_label(effort: &str) -> Option<&'static str> {
    match effort {
        "low" => Some("Low"),
        "medium" => Some("Medium"),
        "high" => Some("High"),
        "xhigh" => Some("Extra high"),
        "max" | "ultra" => Some("More reasoning"),
        _ => None,
    }
}

fn codex_toast_lines(screen: &str) -> impl DoubleEndedIterator<Item = &str> {
    screen.lines().filter_map(|line| {
        line.trim()
            .trim_start_matches('•')
            .trim()
            .strip_prefix("Model changed to ")
    })
}

pub fn codex_change_toast_count(screen: &str) -> usize {
    codex_toast_lines(screen).count()
}

/// Only the bottom-most toast can confirm a change, and only the session-only path.
pub fn codex_change_toast(screen: &str) -> Option<(String, String)> {
    let line = codex_toast_lines(screen).next_back()?;
    let rest = line.strip_suffix(" for this session only")?;
    let mut parts = rest.split_whitespace();
    let slug = parts.next()?;
    let effort = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    Some((slug.to_owned(), effort.to_owned()))
}

/// A change toast is fresh when the screen has more toasts than the baseline
/// read before `s`, or when its bottom-most toast differs from the baseline's
/// (a short terminal can scroll the old toast away as the new one appears).
/// An identical toast that merely stays on screen never confirms.
pub fn codex_fresh_change(
    screen: &str,
    baseline: &str,
    slug: &str,
    effort: Option<&str>,
) -> Option<(String, String)> {
    let more = codex_change_toast_count(screen) > codex_change_toast_count(baseline);
    let replaced = codex_toast_lines(screen).next_back() != codex_toast_lines(baseline).next_back();
    if !more && !replaced {
        return None;
    }
    let applied = codex_change_toast(screen)?;
    (applied.0 == slug && effort.is_none_or(|wanted| applied.1 == wanted)).then_some(applied)
}

/// Claude's cache-invalidation confirmation shown after conversation history exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeSwitchDialog {
    pub model: bool,
    pub options: Vec<(u32, String)>,
}

impl ClaudeSwitchDialog {
    pub fn can_decline(&self) -> bool {
        self.options
            .iter()
            .filter(|(number, _)| *number == 2)
            .count()
            == 1
            && self
                .options
                .iter()
                .any(|(number, label)| *number == 2 && label == "No, go back")
    }
    pub fn matches(&self, target_model: Option<&str>, target_effort: Option<&str>) -> bool {
        if self.options.len() != 2
            || !self.can_decline()
            || self.options[0].0 != 1
            || self.options[1].0 != 2
        {
            return false;
        }
        let Some(target) = self.options[0].1.strip_prefix("Yes, switch to ") else {
            return false;
        };
        if self.model {
            target_model.is_some_and(|model| {
                target
                    .to_ascii_lowercase()
                    .starts_with(&claude_model_label(model).to_ascii_lowercase())
            })
        } else {
            target_effort.is_some_and(|effort| target.eq_ignore_ascii_case(effort))
        }
    }
}

pub fn claude_switch_dialog(screen: &str) -> Option<ClaudeSwitchDialog> {
    let lines: Vec<_> = screen.lines().collect();
    let (index, model) =
        lines
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, line)| match line.trim() {
                "Switch model?" => Some((index, true)),
                "Change effort level?" => Some((index, false)),
                _ => None,
            })?;
    let mut options = Vec::new();
    for raw in &lines[index + 1..] {
        let line = raw.trim().trim_start_matches('❯').trim();
        if line.is_empty() {
            continue;
        }
        if let Some((number, label)) = line.split_once(". ")
            && let Ok(number) = number.parse::<u32>()
        {
            options.push((number, label.trim().to_owned()));
        } else if !options.is_empty() || raw.trim_start().starts_with('❯') {
            // Historical dialog text followed by a composer/result is not a live dialog.
            return None;
        }
    }
    Some(ClaudeSwitchDialog { model, options })
}

/// Claude's confirmation lines after `/model` and `/effort`. Returns the
/// confirmed display label for model lines, and the level for effort lines.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClaudeConfirmation {
    Model(String),
    Effort(String),
    Invalid(String),
    Kept,
}

/// The first result line in `outcome` (the screen after the bridge's own
/// command echo). Reading forward means a desk-side command typed after ours
/// can never stand in for our confirmation.
pub fn claude_confirmation(outcome: &str) -> Option<ClaudeConfirmation> {
    outcome.lines().find_map(claude_result_line)
}

fn claude_result_line(line: &str) -> Option<ClaudeConfirmation> {
    {
        let line = line.trim().trim_start_matches('⎿').trim();
        if line.starts_with("Kept effort level as ") || line.starts_with("Kept model as ") {
            return Some(ClaudeConfirmation::Kept);
        }
        if let Some(rest) = line.strip_prefix("Set model to ") {
            let label = rest.split(" and saved").next().unwrap_or(rest).trim();
            if !label.is_empty() {
                return Some(ClaudeConfirmation::Model(label.to_owned()));
            }
        }
        if let Some(rest) = line.strip_prefix("Set effort level to ") {
            let level = rest.split(' ').next().unwrap_or(rest).trim();
            if !level.is_empty() {
                return Some(ClaudeConfirmation::Effort(level.to_owned()));
            }
        }
        if let Some(rest) = line.strip_prefix("Invalid argument: ") {
            let invalid = rest.split(". Valid options").next().unwrap_or(rest);
            if !invalid.is_empty() {
                return Some(ClaudeConfirmation::Invalid(invalid.to_owned()));
            }
        }
        if line.starts_with("Unknown model") {
            return Some(ClaudeConfirmation::Invalid(line.to_owned()));
        }
    }
    None
}

/// The agent's composer input line parsed from a visible screen. `Draft`
/// covers any text that is not provably the idle placeholder, and `Unknown`
/// any screen without a recognizable input line, so callers fail closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Composer {
    Empty,
    /// Text that matches an idle placeholder. Plain text cannot tell it from
    /// the same words typed by the user, so callers must confirm it is dim.
    Placeholder(String),
    Draft,
    Unknown,
}

// Extracted from installed @openai/codex native binary strings: 0.163.0
// (codex-darwin-arm64/vendor/aarch64-apple-darwin/bin/codex) and 0.50.0
// (codex/vendor/x86_64-apple-darwin/codex/codex). Template-looking tokens
// are literal placeholder text; never match arbitrary substitutions as empty.
const CODEX_PLACEHOLDERS: &[&str] = &[
    "Ask Codex to do anything",
    "Ask a follow-up question",
    "Explain this codebase",
    "Summarize recent commits",
    "Implement {feature}",
    "Find and fix a bug in @filename",
    "Write tests for @filename",
    "Improve documentation in @filename",
];

pub fn composer_state(agent: &str, screen: &str) -> Composer {
    let marker = match agent {
        "claude" => '❯',
        "codex" => '›',
        _ => return Composer::Unknown,
    };
    // The composer is the last line carrying the agent's input marker; slash
    // popup rows above it share the same marker.
    let lines: Vec<&str> = screen.lines().collect();
    let Some(index) = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with(marker))
    else {
        return Composer::Unknown;
    };
    // A multiline draft continues on the lines under the marker (its first
    // line may even be blank); any text before the composer's end is a draft.
    // Blank lines can sit inside a draft, so only a border or footer ends it.
    let continued = lines[index + 1..]
        .iter()
        .map(|line| line.trim())
        .take_while(|line| !is_composer_edge(line))
        .any(|line| !line.is_empty());
    if continued {
        return Composer::Draft;
    }
    let line = lines[index].trim_start();
    let rest = line.trim_start_matches(marker).trim_start();
    if rest.is_empty() {
        return Composer::Empty;
    }
    match agent {
        // Claude's dimmed idle suggestion always reads `Try "…"`.
        "claude" if rest.starts_with("Try \"") && rest.ends_with('"') && rest.len() >= 6 => {
            Composer::Placeholder(rest.to_owned())
        }
        "codex" if CODEX_PLACEHOLDERS.contains(&rest) => Composer::Placeholder(rest.to_owned()),
        _ => Composer::Draft,
    }
}

/// A line that closes the composer: Claude's box border or a blank/footer
/// line follows the input in both agents.
fn is_composer_edge(line: &str) -> bool {
    line.starts_with(['─', '▔', '━', '╰', '└'])
        || line.contains("% left")
        || line.starts_with("? for shortcuts")
        || line.starts_with("⏵⏵")
}

/// Whether the composer line of an ANSI screen renders `placeholder` faint
/// (SGR 2), as both agents draw idle suggestions. Typed input is not faint,
/// so anything unparseable counts as a draft.
pub fn placeholder_is_faint(agent: &str, ansi_screen: &str, placeholder: &str) -> bool {
    let marker = match agent {
        "claude" => '❯',
        "codex" => '›',
        _ => return false,
    };
    let Some(line) = ansi_screen
        .lines()
        .rev()
        .find(|line| strip_ansi(line).trim_start().starts_with(marker))
    else {
        return false;
    };
    let Some(start) = line.find(placeholder) else {
        return false;
    };
    // The SGR state in force where the placeholder starts.
    let mut faint = false;
    let mut rest = &line[..start];
    while let Some(index) = rest.find("\u{1b}[") {
        rest = &rest[index + 2..];
        let Some(end) = rest.find(|c: char| c.is_ascii_alphabetic()) else {
            break;
        };
        if rest.as_bytes()[end] == b'm' {
            let mut params = rest[..end].split(';');
            while let Some(param) = params.next() {
                match param {
                    "" | "0" | "22" => faint = false,
                    "2" => faint = true,
                    // Extended colours carry arguments (`48;2;r;g;b`, `38;5;n`)
                    // whose numbers must not read as attributes.
                    "38" | "48" | "58" => match params.next() {
                        Some("2") => {
                            params.nth(2);
                        }
                        Some("5") => {
                            params.next();
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
        }
        rest = &rest[end + 1..];
    }
    faint
}

fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Well-formed model ids/aliases and efforts accepted by `model.set`.
pub fn valid_set_target(agent: &str, model: Option<&str>, effort: Option<&str>) -> Result<()> {
    // The model is typed into the terminal (`/model <id>`), so it may only
    // hold characters model ids use: no whitespace, line breaks or controls.
    let id_char = |c: char| c.is_ascii_alphanumeric() || "-._:/@[]".contains(c);
    let check = |value: Option<&str>, max: usize| match value {
        None => Ok(()),
        Some(value) if !value.is_empty() && value.len() <= max && value.chars().all(id_char) => {
            Ok(())
        }
        Some(_) => Err("invalid_request"),
    };
    check(model, 128)?;
    match effort {
        None => Ok(()),
        Some(value) if known_effort(agent, value) => Ok(()),
        Some(_) => Err("invalid_effort"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn claude_labels_come_from_the_static_table_or_id_shape() {
        assert_eq!(claude_model_label("opus"), "Opus");
        assert_eq!(claude_display_label("opus"), "Opus 5.5");
        assert_eq!(claude_alias_id("sonnet").as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(claude_alias_id("claude-sonnet-5-5"), None);
        assert!(claude_label_names("Haiku 5.5", "haiku"));
        assert!(claude_label_names("Opus 5.5", "claude-opus-5-5"));
        assert!(!claude_label_names("Haiku 5.5", "sonnet"));
        assert!(!claude_label_names("Haiku 5.5", ""));
        assert_eq!(claude_display_label("fable"), "Fable 5.1");
        assert_eq!(claude_display_label("claude-sonnet-5-5"), "Sonnet 5.5");
        assert_eq!(claude_model_label("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(claude_model_label("claude-fable-5-1"), "Fable 5.1");
        assert_eq!(claude_model_label("claude-haiku-4-5-20251001"), "Haiku 4.5");
        assert_eq!(claude_model_label("claude-sonnet-4-6"), "Sonnet 4.6");
        assert_eq!(
            claude_model_label("gateway/custom-model"),
            "gateway/custom-model"
        );
    }

    #[test]
    fn claude_scan_prefers_assistant_effort_then_api_request() {
        let records = vec![
            json!({"type":"assistant","message":{"model":"claude-opus-5-5"},"effort":"medium"}),
            json!({"type":"api-request","params":{"model":"claude-opus-5-5","output_config":{"effort":"low"}}}),
            json!({"type":"assistant","message":{"model":"claude-sonnet-5-5"},"effort":"high"}),
        ];
        assert_eq!(
            claude_scan(&records),
            Some(("claude-sonnet-5-5".into(), Some("high".into())))
        );
        let legacy = vec![
            json!({"type":"api-request","params":{"output_config":{"effort":"low"}}}),
            json!({"type":"assistant","message":{"model":"claude-opus-4-6"}}),
        ];
        assert_eq!(
            claude_scan(&legacy),
            Some(("claude-opus-4-6".into(), Some("low".into())))
        );
        assert_eq!(
            claude_scan(&[json!({"type":"user","message":{"content":"hi"}})]),
            None
        );
    }

    #[test]
    fn claude_settings_parse_and_effort_resolution() {
        let settings = parse_claude_settings(
            r#"{"model":"opus","effortLevel":null,
                "modelAliases":{"opus":"claude-opus-4-6"},
                "modelSettings":{"claude-sonnet-5-5":{"effortLevel":"high"},
                                 "claude-opus-5-5":{"effortLevel":"medium"}}}"#,
        );
        assert_eq!(settings.model.as_deref(), Some("opus"));
        assert_eq!(settings.effort_for("claude-sonnet-5-5"), Some("high"));
        assert_eq!(settings.effort_for("claude-opus-5-5"), Some("medium"));
        assert_eq!(settings.effort_for("claude-haiku-4-5"), None);
        let override_settings = parse_claude_settings(r#"{"effortLevel":"max"}"#);
        assert_eq!(override_settings.effort_for("anything"), Some("max"));
        assert_eq!(parse_claude_settings("not json").model, None);
    }

    #[test]
    fn codex_scan_takes_the_last_turn_context() {
        let records = vec![
            json!({"type":"turn_context","payload":{"model":"gpt-6.1-sol","effort":"low"}}),
            json!({"type":"response_item","payload":{"type":"message"}}),
            json!({"type":"turn_context","payload":{"model":"gpt-6-luna","effort":"xhigh"}}),
            json!({"type":"turn_context","payload":{"model":"gpt-6-astra"}}),
        ];
        assert_eq!(codex_scan(&records), Some(("gpt-6-astra".into(), None)));
        assert_eq!(
            codex_scan(&records[..2]),
            Some(("gpt-6.1-sol".into(), Some("low".into())))
        );
        assert_eq!(
            codex_scan(&[json!({"type":"session_meta","payload":{}})]),
            None
        );
    }

    #[test]
    fn codex_config_defaults_parse() {
        let text =
            "model = \"gpt-6.1-sol\"\nmodel_reasoning_effort = \"low\"\n[tui]\ntheme = \"dark\"\n";
        assert_eq!(
            parse_codex_config(text),
            (Some("gpt-6.1-sol".into()), Some("low".into()))
        );
        assert_eq!(parse_codex_config("[tui]\n"), (None, None));
        assert_eq!(parse_codex_config("broken ="), (None, None));
    }

    #[test]
    fn catalog_filters_visibility_and_sorts_by_priority() {
        let text = r#"{"models":[
            {"slug":"gpt-6-sol","display_name":"GPT-6-Sol","priority":4,"visibility":"list",
             "supported_reasoning_levels":[{"effort":"low"},{"effort":"high"},{"effort":"weird"}]},
            {"slug":"hidden","display_name":"Hidden","priority":0,"visibility":"hide",
             "supported_reasoning_levels":[]},
            {"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","priority":1,"visibility":"list",
             "supported_reasoning_levels":[]}
        ]}"#;
        let models = parse_codex_catalog(text);
        let slugs: Vec<&str> = models.iter().map(|m| m.slug.as_str()).collect();
        assert_eq!(slugs, vec!["gpt-6.1-sol", "gpt-6-sol"]);
        assert_eq!(models[1].label, "GPT-6-Sol");
        assert_eq!(models[1].efforts, vec!["low", "high"]);
        assert!(parse_codex_catalog("{").is_empty());
        assert!(parse_codex_catalog(r#"{"models":[]}"#).is_empty());
    }

    #[test]
    fn codex_idle_footer_requires_catalog_label_context_delimiter_and_known_effort() {
        let catalog = vec![CodexCatalogModel {
            slug: "gpt-6-sol".into(),
            label: "GPT-6-Sol".into(),
            efforts: vec![],
        }];
        let footer = include_str!("fixtures/codex-idle-footer.txt");
        assert_eq!(
            codex_idle_footer(footer, &catalog),
            Some(("gpt-6-sol".into(), Some("high".into())))
        );
        assert_eq!(
            codex_idle_footer(
                include_str!("fixtures/codex-idle-footer-extra-high.txt"),
                &catalog
            ),
            Some(("gpt-6-sol".into(), Some("xhigh".into())))
        );
        for screen in [
            include_str!("fixtures/codex-idle-footer-unknown.txt").to_owned(),
            footer.replace("GPT-6-Sol", "gpt-6-sol"),
            footer.replace("GPT-6-Sol", "GPT-6-Sol Plus"),
            footer.replace(" · Context ", " · Tokens "),
            footer.replace(" high ·", " impossible ·"),
        ] {
            assert_eq!(codex_idle_footer(&screen, &catalog), None);
        }
        let mut catalog = catalog;
        catalog.push(CodexCatalogModel {
            slug: "ambiguous-slug".into(),
            label: "GPT-6-Sol".into(),
            efforts: vec![],
        });
        assert_eq!(codex_idle_footer(footer, &catalog), None);
    }

    #[test]
    fn real_codex_picker_screens_parse_indented_titles_and_status_suffixes() {
        let models = parse_codex_screen(include_str!("fixtures/codex-model-list.txt"));
        assert!(matches!(models, PickerScreen::ModelList(_)));
        assert_eq!(models.rows().len(), 7);
        assert_eq!(models.cursor(), Some(0));
        assert_eq!(models.row_for_label("GPT-6.1-Sol"), Some(0));
        assert_eq!(models.current_model_row().map(|row| row.number), Some(1));
        let efforts = parse_codex_screen(include_str!("fixtures/codex-effort-list.txt"));
        assert!(
            matches!(&efforts, PickerScreen::EffortList { model_label, .. } if model_label == "GPT-6.1-Sol")
        );
        assert_eq!(efforts.rows().len(), 5);
        assert_eq!(efforts.cursor(), Some(0));
        assert_eq!(efforts.row_for_label("Low"), Some(0));
        assert_eq!(efforts.row_for_label("Medium"), Some(1));
        assert_eq!(efforts.arrow_plan(1), Some(vec!["down"]));
    }

    #[test]
    fn picker_labels_ignore_status_suffixes_in_either_order_on_any_row() {
        for suffix in [
            " (default)",
            " (current)",
            " (default) (current)",
            " (current) (default)",
        ] {
            let label = format!("GPT-6.1-Sol{suffix}");
            assert_eq!(picker_label(&label), "GPT-6.1-Sol");
            let models = parse_codex_screen(&format!(
                "Select Model and Effort\n› 1. {label}  Description"
            ));
            assert_eq!(models.row_for_label("GPT-6.1-Sol"), Some(0));
            assert_eq!(
                models.current_model_row().is_some(),
                suffix.contains("(current)")
            );
            let advanced = parse_codex_screen(&format!(
                "Advanced Reasoning\n  1. Max{suffix}  Description\n› 2. Ultra{suffix}  Description"
            ));
            assert_eq!(advanced.row_for_label("Max"), Some(0));
            assert_eq!(advanced.row_for_label("Ultra"), Some(1));
        }
        assert_eq!(picker_label("A (current) model"), "A (current) model");
    }

    #[test]
    fn codex_screens_parse_rows_cursor_and_titles() {
        let model_screen = "  To get started, describe a task or try one of these commands:\n\
            /model - choose what model and reasoning effort to use\n\
            Select Model and Effort\n\
            › 1. GPT-6.1-Sol (current)  Latest workhorse model\n\
              2. GPT-6-Astra            Frontier intelligence\n\
              7. GPT-5.6-Luna           Older fast model\n\
            enter select · esc back";
        assert_eq!(
            parse_codex_screen(model_screen),
            PickerScreen::ModelList(vec![
                PickerRow {
                    number: 1,
                    label: "GPT-6.1-Sol (current)".into(),
                    selected: true
                },
                PickerRow {
                    number: 2,
                    label: "GPT-6-Astra".into(),
                    selected: false
                },
                PickerRow {
                    number: 7,
                    label: "GPT-5.6-Luna".into(),
                    selected: false
                },
            ])
        );
        let screen = parse_codex_screen(model_screen);
        assert_eq!(screen.current_model_row().map(|row| row.number), Some(1));
        assert_eq!(screen.row_for_label("gpt-6-astra"), Some(1));
        assert_eq!(screen.arrow_plan(2), Some(vec!["down", "down"]));
        assert_eq!(screen.arrow_plan(0), Some(vec![]));

        let effort_screen = "Select Reasoning Level for GPT-6-Luna\n\
            1. Low       Fast responses with lighter reasoning\n\
            2. Medium (default)  Balances speed and reasoning depth\n\
            › 3. High    Greater reasoning depth for complex problems\n\
            4. Extra high  Extra high reasoning depth\n\
            5. More reasoning…  Max consumes usage limits faster";
        assert_eq!(
            parse_codex_screen(effort_screen),
            PickerScreen::EffortList {
                model_label: "GPT-6-Luna".into(),
                rows: vec![
                    PickerRow {
                        number: 1,
                        label: "Low".into(),
                        selected: false
                    },
                    PickerRow {
                        number: 2,
                        label: "Medium (default)".into(),
                        selected: false
                    },
                    PickerRow {
                        number: 3,
                        label: "High".into(),
                        selected: true
                    },
                    PickerRow {
                        number: 4,
                        label: "Extra high".into(),
                        selected: false
                    },
                    PickerRow {
                        number: 5,
                        label: "More reasoning…".into(),
                        selected: false
                    },
                ]
            }
        );
        let screen = parse_codex_screen(effort_screen);
        assert_eq!(screen.row_for_label("More reasoning…"), Some(4));
        assert_eq!(screen.arrow_plan(4), Some(vec!["down", "down"]));

        let advanced = "Advanced Reasoning\n\
            ⚠ Consumes usage limits faster\n\
            › 1. Max  For difficult problems when quality matters more than speed\n\
              2. Ultra  Maximum reasoning with automatic task delegation";
        assert_eq!(
            parse_codex_screen(advanced),
            PickerScreen::Advanced(vec![
                PickerRow {
                    number: 1,
                    label: "Max".into(),
                    selected: true
                },
                PickerRow {
                    number: 2,
                    label: "Ultra".into(),
                    selected: false
                },
            ])
        );
        assert!(parse_codex_screen("› Ask Codex to do anything").is_composer());
    }

    #[test]
    fn codex_confirmation_requires_a_new_bottommost_session_only_toast() {
        let old = "• Model changed to gpt-6.1-sol medium for this session only";
        let new = "• Model changed to gpt-6-luna medium for this session only";
        let both = format!("{old}\n{new}");
        assert_eq!(codex_change_toast_count(&both), 2);
        assert_eq!(
            codex_fresh_change(&both, old, "gpt-6-luna", Some("medium")),
            Some(("gpt-6-luna".into(), "medium".into()))
        );
        // The old toast scrolled away as the new one appeared: still fresh.
        assert_eq!(
            codex_fresh_change(new, old, "gpt-6-luna", Some("medium")),
            Some(("gpt-6-luna".into(), "medium".into()))
        );
        // The same toast merely remaining on screen is not a confirmation.
        assert_eq!(codex_fresh_change(new, new, "gpt-6-luna", Some("medium")), None);
        assert_eq!(codex_fresh_change(&both, old, "gpt-6.1-sol", None), None);
        assert_eq!(
            codex_fresh_change(&both, old, "gpt-6-luna", Some("high")),
            None
        );
        assert_eq!(
            codex_change_toast("• Model changed to gpt-6-luna medium"),
            None
        );
        assert_eq!(
            codex_change_toast(&format!("{new}\n• Model changed to gpt-6-luna medium")),
            None
        );
    }

    #[test]
    fn claude_cache_switch_dialogs_match_real_screens_and_fail_closed() {
        let effort_screen = include_str!("fixtures/claude-effort-confirm.txt");
        let effort = claude_switch_dialog(effort_screen).unwrap();
        assert!(effort.matches(None, Some("medium")));
        assert!(!effort.matches(None, Some("high")));
        assert!(!effort.matches(Some("haiku"), None));
        let model_screen = include_str!("fixtures/claude-model-confirm.txt");
        assert!(
            claude_switch_dialog(model_screen)
                .unwrap()
                .matches(Some("haiku"), None)
        );
        assert!(
            !claude_switch_dialog(model_screen)
                .unwrap()
                .matches(Some("opus"), None)
        );
        assert!(
            claude_switch_dialog(&model_screen.replace("Haiku", "HAIKU"))
                .unwrap()
                .matches(Some("haiku"), None)
        );
        assert!(
            !claude_switch_dialog(&effort_screen.replace("No, go back", "Other action"))
                .unwrap()
                .matches(None, Some("medium"))
        );
        assert!(
            !claude_switch_dialog(&format!("{effort_screen}  3. Unexpected option"))
                .unwrap()
                .matches(None, Some("medium"))
        );
        assert!(claude_switch_dialog(&format!("{effort_screen}\n❯ new draft")).is_none());
        assert!(
            claude_switch_dialog(&effort_screen.replace("Change effort level?", "Other dialog?"))
                .is_none()
        );
        assert_eq!(
            claude_confirmation("⎿ Kept effort level as low"),
            Some(ClaudeConfirmation::Kept)
        );
        assert_eq!(
            claude_confirmation("⎿ Kept model as Sonnet 5.5"),
            Some(ClaudeConfirmation::Kept)
        );
    }

    #[test]
    fn change_toast_and_claude_confirmations_parse() {
        assert_eq!(
            codex_change_toast("• Model changed to gpt-6-luna high for this session only"),
            Some(("gpt-6-luna".into(), "high".into()))
        );
        assert_eq!(
            codex_change_toast("• Model changed to gpt-6-luna high"),
            None
        );
        assert_eq!(codex_change_toast("nothing happened"), None);

        assert_eq!(
            claude_confirmation(
                "  ⎿  Set model to Sonnet 5.5 and saved as your default for new sessions"
            ),
            Some(ClaudeConfirmation::Model("Sonnet 5.5".into()))
        );
        assert_eq!(
            claude_confirmation(
                "⎿ Set effort level to high (saved as your default for new sessions)"
            ),
            Some(ClaudeConfirmation::Effort("high".into()))
        );
        assert_eq!(
            claude_confirmation("⎿ Invalid argument: bogus. Valid options are: low, medium, high"),
            Some(ClaudeConfirmation::Invalid("bogus".into()))
        );
        assert_eq!(
            claude_confirmation("⎿ Unknown model 'zzz' isn't available"),
            Some(ClaudeConfirmation::Invalid(
                "Unknown model 'zzz' isn't available".into()
            ))
        );
        assert_eq!(claude_confirmation("plain line"), None);
    }

    #[test]
    fn picker_parses_two_digit_rows_and_rejects_ambiguous_cursors() {
        let screen =
            parse_codex_screen("Select Model and Effort\n  9. Ninth\n› 10. Tenth\n  11. Eleventh");
        assert_eq!(screen.rows().len(), 3);
        assert_eq!(screen.rows()[1].number, 10);
        assert_eq!(screen.arrow_plan(2), Some(vec!["down"]));
        let ambiguous = parse_codex_screen("Select Model and Effort\n› 9. Ninth\n› 10. Tenth");
        assert_eq!(ambiguous.cursor(), None);
        assert_eq!(ambiguous.arrow_plan(0), None);
    }

    #[test]
    fn placeholder_counts_only_when_rendered_faint() {
        // Real Codex 0.163 composer line (pane.read with strip_ansi false).
        let dim = "\u{1b}[0m\u{1b}[1m\u{1b}[48;2;65;69;76m›\u{1b}[0m\u{1b}[48;2;65;69;76m \u{1b}[0m\u{1b}[2m\u{1b}[48;2;65;69;76mAsk Codex to do anything\u{1b}[0m";
        assert!(placeholder_is_faint("codex", dim, "Ask Codex to do anything"));
        // The same words typed by the user render at normal intensity.
        let typed = "\u{1b}[0m\u{1b}[1m\u{1b}[2m›\u{1b}[0m\u{1b}[48;2;65;69;76m Explain this codebase\u{1b}[0m";
        assert!(!placeholder_is_faint("codex", typed, "Explain this codebase"));
        assert!(!placeholder_is_faint("codex", "› Explain this codebase", "Explain this codebase"));
        let claude = "\u{1b}[0m❯ \u{1b}[2mTry \"fix lint errors\"\u{1b}[22m";
        assert!(placeholder_is_faint("claude", claude, "Try \"fix lint errors\""));
        assert!(!placeholder_is_faint("claude", "❯ Try \"fix lint errors\"", "Try \"fix lint errors\""));
    }

    #[test]
    fn composer_state_treats_placeholders_as_empty_and_fails_closed() {
        assert_eq!(
            composer_state("claude", "…\n❯ Try \"fix lint errors\"\n"),
            Composer::Placeholder("Try \"fix lint errors\"".into())
        );
        assert_eq!(composer_state("claude", "…\n❯\n"), Composer::Empty);
        assert_eq!(
            composer_state("claude", "…\n❯ /model sonnet\n"),
            Composer::Draft
        );
        assert_eq!(
            composer_state("claude", "…\n❯ half-written draft\n"),
            Composer::Draft
        );
        // A slash popup above the composer: the last marker line is the input.
        assert_eq!(
            composer_state(
                "claude",
                "❯ /model  Set the AI model\n…\n❯ Try \"fix lint errors\"\n"
            ),
            Composer::Placeholder("Try \"fix lint errors\"".into())
        );
        assert_eq!(
            composer_state("claude", "no marker at all"),
            Composer::Unknown
        );
        assert_eq!(
            composer_state(
                "codex",
                "› Ask Codex to do anything\nlow · Context 100% left"
            ),
            Composer::Placeholder("Ask Codex to do anything".into())
        );
        assert_eq!(
            composer_state("codex", "› Ask a follow-up question"),
            Composer::Placeholder("Ask a follow-up question".into())
        );
        assert_eq!(composer_state("codex", "› hello draft"), Composer::Draft);
        // Multiline drafts whose first line is blank.
        assert_eq!(
            composer_state("claude", "─────\n❯ \n  second line of a draft\n─────\n  ⏵⏵ auto mode on"),
            Composer::Draft
        );
        assert_eq!(
            composer_state("codex", "›\n  second line\n\n  GPT-6.1-Sol low · Context 100% left"),
            Composer::Draft
        );
        // Several blank lines before the draft's first text.
        assert_eq!(
            composer_state("claude", "─────\n❯ \n\n\n  text after two newlines\n─────"),
            Composer::Draft
        );
        assert_eq!(
            composer_state("codex", "›\n\n\n  later text\n\n  GPT-6.1-Sol low · Context 100% left"),
            Composer::Draft
        );
        // Real empty composers: the box border or a blank line follows.
        assert_eq!(
            composer_state("claude", "─────\n❯ \n─────\n  ⏵⏵ auto mode on"),
            Composer::Empty
        );
        assert_eq!(
            composer_state("codex", "› Ask Codex to do anything\n   \n  GPT-6.1-Sol low · Context 100% left\n  ? for shortcuts"),
            Composer::Placeholder("Ask Codex to do anything".into())
        );
        assert_eq!(composer_state("codex", "no marker"), Composer::Unknown);
        assert_eq!(composer_state("agy", "›"), Composer::Unknown);
    }

    #[test]
    fn codex_rotating_placeholders_need_a_style_check_and_substituted_drafts_do_not() {
        for placeholder in CODEX_PLACEHOLDERS {
            assert_eq!(
                composer_state("codex", &format!("› {placeholder}\nContext 100% left")),
                Composer::Placeholder((*placeholder).to_owned()),
                "{placeholder}"
            );
        }
        for draft in [
            "Implement a model picker",
            "Find and fix a bug in src/main.rs",
            "Write tests for @filename please",
            "Explain this codebase and fix it",
        ] {
            assert_eq!(
                composer_state("codex", &format!("› {draft}")),
                Composer::Draft
            );
        }
    }

    #[test]
    fn set_target_validation() {
        assert!(valid_set_target("claude", Some("opus"), Some("max")).is_ok());
        assert!(valid_set_target("claude", None, Some("auto")).is_ok());
        assert_eq!(
            valid_set_target("claude", None, Some("turbo")).unwrap_err(),
            "invalid_effort"
        );
        assert_eq!(
            valid_set_target("codex", Some(""), None).unwrap_err(),
            "invalid_request"
        );
        assert!(valid_set_target("codex", Some("gpt-6-luna"), Some("ultra")).is_ok());
        for id in ["claude-opus-5-5[1m]", "default", "us.anthropic.claude-opus-5-5", "gateway/model@v1"] {
            assert!(valid_set_target("claude", Some(id), None).is_ok(), "{id}");
        }
        for id in ["sonnet\nadditional text", "sonnet more", "opus\r", "a\u{1b}[2J", "opus\t"] {
            assert_eq!(
                valid_set_target("claude", Some(id), None).unwrap_err(),
                "invalid_request",
                "{id:?}"
            );
        }
    }

    #[test]
    fn claude_scan_uses_latest_request_effort_when_records_omit_it() {
        let records = vec![
            serde_json::json!({"type":"api-request","params":{"output_config":{"effort":"low"}}}),
            serde_json::json!({"type":"assistant","message":{"model":"claude-opus-5-5"}}),
            serde_json::json!({"type":"api-request","params":{"output_config":{"effort":"high"}}}),
            serde_json::json!({"type":"assistant","message":{"model":"claude-opus-5-5"}}),
        ];
        assert_eq!(
            claude_scan(&records),
            Some(("claude-opus-5-5".into(), Some("high".into())))
        );
    }

    #[test]
    fn claude_scan_ignores_sidechain_and_meta_records() {
        let records = vec![
            serde_json::json!({"type":"assistant","message":{"model":"claude-opus-5-5"},"effort":"high"}),
            serde_json::json!({"type":"assistant","isSidechain":true,"message":{"model":"claude-haiku-5-5"},"effort":"low"}),
            serde_json::json!({"type":"api-request","isSidechain":true,"params":{"output_config":{"effort":"low"}}}),
            serde_json::json!({"type":"assistant","isMeta":true,"message":{"model":"claude-sonnet-5-5"}}),
        ];
        assert_eq!(
            claude_scan(&records),
            Some(("claude-opus-5-5".into(), Some("high".into())))
        );
    }

    #[test]
    fn claude_scan_applies_desk_side_local_commands_until_the_next_reply() {
        let stdout = |text: &str| {
            serde_json::json!({"type":"user","message":{"role":"user",
                "content":format!("<local-command-stdout>{text}</local-command-stdout>")}})
        };
        let mut records = vec![
            serde_json::json!({"type":"assistant","message":{"model":"claude-opus-5-5"},"effort":"medium"}),
            stdout("Set model to Haiku 5.5 and saved as your default for new sessions"),
        ];
        assert_eq!(
            claude_scan(&records),
            Some(("claude-haiku-5-5".into(), Some("medium".into())))
        );
        records.push(stdout(
            "Set effort level to low (saved as your default for new sessions): Quick",
        ));
        assert_eq!(
            claude_scan(&records),
            Some(("claude-haiku-5-5".into(), Some("low".into())))
        );
        records.push(stdout("Kept effort level as low"));
        assert_eq!(
            claude_scan(&records),
            Some(("claude-haiku-5-5".into(), Some("low".into())))
        );
        records.push(serde_json::json!({"type":"assistant","message":{"model":"claude-haiku-5-5"},"effort":"low"}));
        assert_eq!(
            claude_scan(&records),
            Some(("claude-haiku-5-5".into(), Some("low".into())))
        );
    }

    #[test]
    fn claude_local_model_confirmations_keep_their_version() {
        let change = |text: &str| {
            claude_local_command_change(&serde_json::json!(format!(
                "<local-command-stdout>{text}</local-command-stdout>"
            )))
        };
        assert!(matches!(
            change("Set model to Haiku 4.5 and saved as your default for new sessions"),
            Some(LocalChange::Model(id)) if id == "claude-haiku-4-5"
        ));
        assert!(matches!(
            change("Set model to Default (recommended)"),
            Some(LocalChange::Model(id)) if id == "default"
        ));
        assert!(matches!(
            change("Set model to Opus"),
            Some(LocalChange::Model(id)) if id == "opus"
        ));
        assert_eq!(claude_display_label("claude-haiku-4-5"), "Haiku 4.5");
        assert_eq!(claude_display_label("default"), "Default");
    }

    #[test]
    fn claude_settings_effort_resolves_alias_to_full_id() {
        let settings = parse_claude_settings(
            r#"{"model":"sonnet","modelSettings":{"claude-sonnet-5-5":{"effortLevel":"xhigh"}}}"#,
        );
        assert_eq!(settings.effort_for("sonnet"), Some("xhigh"));
        assert_eq!(settings.effort_for("claude-sonnet-5-5"), Some("xhigh"));
        assert_eq!(settings.effort_for("opus"), None);
        let configured = parse_claude_settings(
            r#"{"model":"opus","modelAliases":{"opus":"claude-opus-4-6"},
                "modelSettings":{"claude-opus-4-6":{"effortLevel":"high"}}}"#,
        );
        assert_eq!(configured.effort_for("opus"), Some("high"));
    }

}
