//! Pure detection of pending questions from transcripts and rollouts.
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
            // Expiry/skip at turn end is inferred; those paths were not captured.
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
