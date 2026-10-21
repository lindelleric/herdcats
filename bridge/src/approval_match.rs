//! Pure correlation of permission spool records with pending transcript calls.
use crate::hooks::Agent;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct PendingRequest {
    pub agent: Agent,
    pub session_id: String,
    pub request_id: String,
    pub tool_name: String,
    pub tool_input: Value,
    pub justification: Option<String>,
    pub suggestions: Option<Value>,
    pub recorded_at: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolution {
    Pending,
    Resolved,
    Aborted,
}

fn nonempty(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.is_empty())
}

fn request(record: &Value, agent: Agent) -> Option<PendingRequest> {
    if record["v"].as_u64() != Some(1)
        || record["event"] != "PermissionRequest"
        || record["agent"].as_str() != Some(agent.name())
    {
        return None;
    }
    let payload = record.get("payload")?;
    let tool_input = payload.get("tool_input")?.clone();
    if !tool_input.is_object() {
        return None;
    }
    Some(PendingRequest {
        agent,
        session_id: nonempty(&record["session_id"])?.into(),
        request_id: String::new(),
        tool_name: nonempty(&payload["tool_name"])?.into(),
        justification: if agent == Agent::Codex {
            tool_input["description"].as_str().map(str::to_string)
        } else {
            None
        },
        tool_input,
        suggestions: if agent == Agent::Claude {
            payload.get("permission_suggestions").cloned()
        } else {
            None
        },
        recorded_at: record["t"].as_f64()?,
    })
}

fn blocks(record: &Value) -> impl Iterator<Item = &Value> {
    record["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
}
fn tool_result(block: &Value, id: &str) -> bool {
    block["type"] == "tool_result" && block["tool_use_id"].as_str() == Some(id)
}

pub fn match_claude(record: &Value, transcript: &[Value]) -> Option<PendingRequest> {
    let mut request = request(record, Agent::Claude)?;
    let content: Vec<_> = transcript
        .iter()
        .flat_map(|r| blocks(r).map(move |b| (r, b)))
        .collect();
    let mut candidate = None;
    for (i, (r, block)) in content.iter().enumerate() {
        if r["type"] != "assistant"
            || block["type"] != "tool_use"
            || block["name"].as_str() != Some(&request.tool_name)
            || block.get("input") != Some(&request.tool_input)
        {
            continue;
        }
        let id = nonempty(&block["id"])?;
        if content[i + 1..].iter().any(|(_, b)| tool_result(b, id)) {
            continue;
        }
        if candidate.replace(id).is_some() {
            return None;
        }
    }
    request.request_id = candidate?.into();
    Some(request)
}

pub fn claude_resolution(request_id: &str, transcript: &[Value]) -> Resolution {
    if transcript
        .iter()
        .flat_map(blocks)
        .any(|b| tool_result(b, request_id))
    {
        Resolution::Resolved
    } else {
        Resolution::Pending
    }
}

fn call_output(record: &Value, id: &str) -> bool {
    record["type"] == "response_item"
        && matches!(
            record["payload"]["type"].as_str(),
            Some("custom_tool_call_output" | "function_call_output")
        )
        && record["payload"]["call_id"].as_str() == Some(id)
}

fn event_turn(payload: &Value) -> Option<&str> {
    payload["turn_id"]
        .as_str()
        .or_else(|| payload["root_turn_id"].as_str())
}

pub fn match_codex(record: &Value, rollout: &[Value]) -> Option<PendingRequest> {
    let mut request = request(record, Agent::Codex)?;
    let turn = nonempty(&record["payload"]["turn_id"])?;
    let command = request.tool_input["command"].as_str()?;
    let quoted = serde_json::to_string(command).ok()?;
    let mut current_turn = None;
    let mut candidate = None;
    for (i, r) in rollout.iter().enumerate() {
        let payload = &r["payload"];
        if r["type"] == "event_msg" {
            if let Some(turn) = event_turn(payload) {
                current_turn = Some(turn);
            }
        }
        if current_turn != Some(turn)
            || r["type"] != "response_item"
            || payload["type"] != "custom_tool_call"
            || !payload["input"]
                .as_str()
                .is_some_and(|input| input.contains(&quoted))
        {
            continue;
        }
        let id = nonempty(&payload["call_id"])?;
        if rollout[i + 1..].iter().any(|r| call_output(r, id)) {
            continue;
        }
        if candidate.replace(id).is_some() {
            return None;
        }
    }
    request.request_id = candidate?.into();
    Some(request)
}

pub fn codex_resolution(request_id: &str, turn_id: &str, rollout: &[Value]) -> Resolution {
    // Output is definitive even if the same turn also carries an abort event.
    if rollout.iter().any(|r| call_output(r, request_id)) {
        Resolution::Resolved
    } else if rollout.iter().any(|r| {
        r["type"] == "event_msg"
            && r["payload"]["type"] == "turn_aborted"
            && event_turn(&r["payload"]) == Some(turn_id)
    }) {
        Resolution::Aborted
    } else {
        Resolution::Pending
    }
}

/// Correlate a session's spool records, retaining the newest metadata per call.
pub fn match_claude_records(records: &[Value], transcript: &[Value]) -> Vec<PendingRequest> {
    match_records(records, transcript, match_claude)
}

/// Correlate a session's spool records, retaining the newest metadata per call.
pub fn match_codex_records(records: &[Value], rollout: &[Value]) -> Vec<PendingRequest> {
    match_records(records, rollout, match_codex)
}

fn match_records(
    records: &[Value],
    history: &[Value],
    matcher: fn(&Value, &[Value]) -> Option<PendingRequest>,
) -> Vec<PendingRequest> {
    let mut latest = std::collections::BTreeMap::<String, PendingRequest>::new();
    for record in records {
        let Some(request) = matcher(record, history) else {
            continue;
        };
        let entry = latest.entry(request.request_id.clone());
        match entry {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(request);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                // Replacement on equality gives the later input record precedence.
                if request.recorded_at >= entry.get().recorded_at {
                    entry.insert(request);
                }
            }
        }
    }
    let mut requests: Vec<_> = latest.into_values().collect();
    requests.sort_by(|a, b| a.recorded_at.total_cmp(&b.recorded_at));
    requests
}
