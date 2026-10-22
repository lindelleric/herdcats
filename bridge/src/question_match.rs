//! Pure detection of pending questions from Claude hook records and native history.
use crate::approval_match::utc_timestamp;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestionKind {
    ClaudeAsk,
    CodexPlan,
    CodexAsync,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionOption {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: Option<String>,
    pub header: Option<String>,
    pub text: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: bool,
    pub allows_other: bool,
    pub secret: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingQuestion {
    pub request_id: String,
    pub kind: QuestionKind,
    pub turn_id: Option<String>,
    pub questions: Vec<Question>,
}

fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.trim().is_empty())
}

fn flag(value: &Value, key: &str) -> Option<bool> {
    match value.get(key) {
        None => Some(false),
        Some(v) => v.as_bool(),
    }
}

fn question(value: &Value, kind: QuestionKind) -> Option<Question> {
    let asynchronous = kind == QuestionKind::CodexAsync;
    let empty = Vec::new();
    let option_values = match value.get("options") {
        None if asynchronous => &empty,
        Some(v) => v.as_array()?,
        None => return None,
    };
    let options = option_values
        .iter()
        .map(|o| {
            Some(QuestionOption {
                label: text(if asynchronous { o } else { &o["label"] })?.into(),
                description: if asynchronous {
                    None
                } else {
                    Some(o["description"].as_str()?.into())
                },
            })
        })
        .collect::<Option<Vec<_>>>()?;
    if !asynchronous && options.is_empty() {
        return None;
    }
    Some(Question {
        id: if kind == QuestionKind::CodexPlan {
            Some(text(&value["id"])?.into())
        } else {
            None
        },
        header: if asynchronous {
            None
        } else {
            Some(text(&value["header"])?.into())
        },
        text: text(&value[if asynchronous { "title" } else { "question" }])?.into(),
        options,
        multi_select: kind == QuestionKind::ClaudeAsk && value["multiSelect"].as_bool()?,
        allows_other: true,
        secret: if kind == QuestionKind::CodexPlan {
            // isOther controls a tool option; notes remain available regardless.
            flag(value, "isOther")?;
            flag(value, "isSecret")?
        } else {
            false
        },
    })
}

fn questions(value: &Value, kind: QuestionKind) -> Option<Vec<Question>> {
    let values = value["questions"].as_array()?;
    if values.is_empty() {
        return None;
    }
    values.iter().map(|q| question(q, kind)).collect()
}

/// Claude writes AskUserQuestion tool_use only when the picker finishes. The
/// recording hook is the pending source; transcript pairs retire spool records.
pub fn pending_claude_spool_questions(
    records: &[Value],
    transcript: &[Value],
) -> Vec<PendingQuestion> {
    let main = |r: &&Value| r["isSidechain"] != true && r["isMeta"] != true;
    let results: HashSet<_> = transcript
        .iter()
        .filter(main)
        .filter(|r| r["type"] == "user")
        .flat_map(|r| {
            r["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| {
                    if b["type"] != "tool_result" {
                        return None;
                    }
                    Some((text(&r["sessionId"])?, text(&b["tool_use_id"])?))
                })
        })
        .collect();
    let mut answered: Vec<_> = transcript
        .iter()
        .filter(main)
        .filter(|r| r["type"] == "assistant")
        .flat_map(|r| {
            r["message"]["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| {
                    let session = text(&r["sessionId"])?;
                    if b["type"] == "tool_use"
                        && b["name"] == "AskUserQuestion"
                        && results.contains(&(session, text(&b["id"])?))
                    {
                        Some((utc_timestamp(&r["timestamp"])?, session, &b["input"]))
                    } else {
                        None
                    }
                })
        })
        .collect();
    answered.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut candidates: Vec<_> = records
        .iter()
        .filter_map(|r| {
            let payload = &r["payload"];
            let session = text(&r["session_id"])?;
            if r["event"] != "PermissionRequest"
                || r["agent"] != "claude"
                || payload["tool_name"] != "AskUserQuestion"
                || payload["session_id"] != session
                || r["truncated"] == true
                || payload["truncated"] == true
            {
                return None;
            }
            let prompt = text(&payload["prompt_id"])?;
            let time = r["t"].as_f64()?;
            let millis = (time * 1000.0).floor();
            if !millis.is_finite() || millis < 0.0 || millis >= u64::MAX as f64 {
                return None;
            }
            let questions = questions(&payload["tool_input"], QuestionKind::ClaudeAsk)?;
            Some((
                time,
                session,
                &payload["tool_input"],
                PendingQuestion {
                    request_id: format!("ask-{prompt}-{}", millis as u64),
                    kind: QuestionKind::ClaudeAsk,
                    turn_id: None,
                    questions,
                },
            ))
        })
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates
        .into_iter()
        .filter_map(|(recorded_at, session, input, request)| {
            // Esc cancel also writes tool_use + tool_result, so it counts as answered.
            // Pair oldest identical calls one-to-one only near the hook time.
            // Older answered calls may predate hook installation; they must not
            // retire a new identical request. Allow 2s clock skew and at most 120s delay.
            if let Some(index) = answered.iter().position(|(timestamp, s, i)| {
                *s == session
                    && *i == input
                    && *timestamp <= recorded_at + 2.0
                    && recorded_at - *timestamp <= 120.0
            }) {
                answered.remove(index);
                None
            } else {
                Some(request)
            }
        })
        .collect()
}

/// Retained for offline transcript consumers/tests; live pickers require the hook spool.
pub fn pending_claude_questions(transcript: &[Value]) -> Vec<PendingQuestion> {
    let mut pending = Vec::new();
    for record in transcript {
        if record["isSidechain"] == true || record["isMeta"] == true {
            continue;
        }
        let Some(blocks) = record["message"]["content"].as_array() else {
            continue;
        };
        for block in blocks {
            if record["type"] == "user" && block["type"] == "tool_result" {
                if let Some(id) = text(&block["tool_use_id"]) {
                    pending.retain(|q: &PendingQuestion| q.request_id != id);
                }
            } else if record["type"] == "assistant"
                && block["type"] == "tool_use"
                && block["name"] == "AskUserQuestion"
            {
                if let (Some(id), Some(questions)) = (
                    text(&block["id"]),
                    questions(&block["input"], QuestionKind::ClaudeAsk),
                ) {
                    pending.push(PendingQuestion {
                        request_id: id.into(),
                        kind: QuestionKind::ClaudeAsk,
                        turn_id: None,
                        questions,
                    });
                }
            }
        }
    }
    pending
}

fn turn(payload: &Value) -> Option<&str> {
    text(&payload["turn_id"]).or_else(|| text(&payload["root_turn_id"]))
}

fn apply_reply(
    payload: &Value,
    pending: &mut Vec<PendingQuestion>,
    answered: &mut HashMap<String, HashSet<u64>>,
) {
    if payload["type"] != "message" || payload["role"] != "user" {
        return;
    }
    for block in payload["content"].as_array().into_iter().flatten() {
        let Some(body) = block["text"].as_str() else {
            continue;
        };
        let Some((_, rest)) = body.split_once("<send_user_message_question_reply>") else {
            continue;
        };
        let Some((json, _)) = rest.split_once("</send_user_message_question_reply>") else {
            continue;
        };
        let Ok(Value::Array(replies)) = serde_json::from_str::<Value>(json.trim()) else {
            continue;
        };
        for reply in replies {
            if !reply["answer"].is_string() {
                continue;
            }
            let Some(link) = reply["questionItemId"].as_str() else {
                continue;
            };
            let Ok(Value::Array(link)) = serde_json::from_str::<Value>(link) else {
                continue;
            };
            if link.len() != 3 || link[0] != "request_user_input_async" {
                continue;
            }
            let (Some(id), Some(index)) = (text(&link[1]), link[2].as_u64()) else {
                continue;
            };
            if let Some(q) = pending.iter().find(|q| {
                q.kind == QuestionKind::CodexAsync
                    && q.request_id == id
                    && index < q.questions.len() as u64
            }) {
                let indices = answered.entry(id.into()).or_default();
                indices.insert(index);
                if indices.len() == q.questions.len() {
                    pending.retain(|q| q.request_id != id);
                    answered.remove(id);
                }
            }
        }
    }
}

pub fn pending_codex_questions(rollout: &[Value]) -> Vec<PendingQuestion> {
    let mut pending = Vec::new();
    let mut current_turn = None;
    let mut answered = HashMap::new();
    for record in rollout {
        let payload = &record["payload"];
        if record["type"] == "event_msg" {
            if payload["type"] == "task_started" {
                current_turn = turn(payload).map(str::to_owned);
            }
            // observed 2026-10-22 (Codex 0.163.0): async skip writes no reply
            // record. The model keeps sleeping and the call remains pending until
            // turn end; the missing banner/picker prevents a card meanwhile.
            // Expiry via turn end remains inferred rather than captured.
            if matches!(
                payload["type"].as_str(),
                Some("task_complete" | "turn_aborted")
            ) {
                if let Some(ended) = turn(payload) {
                    pending.retain(|q: &PendingQuestion| q.turn_id.as_deref() != Some(ended));
                    if current_turn.as_deref() == Some(ended) {
                        current_turn = None;
                    }
                }
            }
            continue;
        }
        if record["type"] != "response_item" {
            continue;
        }
        if payload["type"] == "function_call_output" {
            if let Some(id) = text(&payload["call_id"]) {
                pending.retain(|q| q.kind != QuestionKind::CodexPlan || q.request_id != id);
            }
        }
        apply_reply(payload, &mut pending, &mut answered);
        if payload["type"] != "function_call" {
            continue;
        }
        let kind = match payload["name"].as_str() {
            Some("request_user_input") => QuestionKind::CodexPlan,
            Some("request_user_input_async") => QuestionKind::CodexAsync,
            _ => continue,
        };
        let Some(id) = text(&payload["call_id"]) else {
            continue;
        };
        let Some(args) = payload["arguments"].as_str() else {
            continue;
        };
        let Ok(args) = serde_json::from_str::<Value>(args) else {
            continue;
        };
        let Some(questions) = questions(&args, kind) else {
            continue;
        };
        pending.push(PendingQuestion {
            request_id: id.into(),
            kind,
            turn_id: text(&payload["internal_chat_message_metadata_passthrough"]["turn_id"])
                .map(str::to_owned)
                .or_else(|| current_turn.clone()),
            questions,
        });
    }
    pending
}

#[cfg(test)]
mod tests;
