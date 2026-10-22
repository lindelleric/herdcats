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
    pub turn_id: Option<String>,
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
        turn_id: if agent == Agent::Codex {
            nonempty(&payload["turn_id"]).map(str::to_owned)
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

/// Parse the UTC timestamp shape emitted by transcripts and rollouts, including
/// fractional seconds. Reject malformed dates rather than normalizing them.
pub(crate) fn utc_timestamp(value: &Value) -> Option<f64> {
    fn decimal(s: &str) -> Option<i64> {
        if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        s.parse().ok()
    }
    let utc = value.as_str()?.strip_suffix('Z')?;
    let (whole, fraction) = if let Some((whole, fraction)) = utc.split_once('.') {
        if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        (whole, format!("0.{fraction}").parse::<f64>().ok()?)
    } else {
        (utc, 0.0)
    };
    if whole.len() != 19
        || whole.get(4..5)? != "-"
        || whole.get(7..8)? != "-"
        || whole.get(10..11)? != "T"
        || whole.get(13..14)? != ":"
        || whole.get(16..17)? != ":"
    {
        return None;
    }
    let year = decimal(whole.get(0..4)?)?;
    let month = decimal(whole.get(5..7)?)?;
    let day = decimal(whole.get(8..10)?)?;
    let hour = decimal(whole.get(11..13)?)?;
    let minute = decimal(whole.get(14..16)?)?;
    let second = decimal(whole.get(17..19)?)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        _ => return None,
    };
    if !(1..=days_in_month).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    // Gregorian days-from-civil, the inverse of codex::civil_from_days.
    let adjusted_year = year - i64::from(month <= 2);
    let era = adjusted_year.div_euclid(400);
    let year_of_era = adjusted_year - era * 400;
    let march_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * march_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146097 + day_of_era - 719468;
    Some((days * 86400 + hour * 3600 + minute * 60 + second) as f64 + fraction)
}

pub fn match_claude(record: &Value, transcript: &[Value]) -> Option<PendingRequest> {
    let mut candidates = claude_candidates(record, transcript, true)?;
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}

fn claude_candidates(
    record: &Value,
    transcript: &[Value],
    pending_only: bool,
) -> Option<Vec<PendingRequest>> {
    let mut request = request(record, Agent::Claude)?;
    let content: Vec<_> = transcript
        .iter()
        .flat_map(|r| blocks(r).map(move |b| (r, b)))
        .collect();
    let mut candidates = Vec::new();
    for (i, (r, block)) in content.iter().enumerate() {
        if r["type"] != "assistant"
            || block["type"] != "tool_use"
            || block["name"].as_str() != Some(&request.tool_name)
            || block.get("input") != Some(&request.tool_input)
        {
            continue;
        }
        let Some(timestamp) = utc_timestamp(&r["timestamp"]) else {
            continue;
        };
        if request.recorded_at < timestamp {
            continue;
        }
        let id = nonempty(&block["id"])?;
        if pending_only && content[i + 1..].iter().any(|(_, b)| tool_result(b, id)) {
            continue;
        }
        request.request_id = id.into();
        candidates.push(request.clone());
    }
    Some(candidates)
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
    let mut candidates = codex_candidates(record, rollout, true)?;
    if candidates.len() == 1 {
        candidates.pop()
    } else {
        None
    }
}

fn codex_candidates(
    record: &Value,
    rollout: &[Value],
    pending_only: bool,
) -> Option<Vec<PendingRequest>> {
    let mut request = request(record, Agent::Codex)?;
    let turn = nonempty(&record["payload"]["turn_id"])?;
    let command = request.tool_input["command"].as_str()?;
    let quoted = serde_json::to_string(command).ok()?;
    let mut current_turn = None;
    let mut candidates = Vec::new();
    for (i, r) in rollout.iter().enumerate() {
        let payload = &r["payload"];
        if r["type"] == "event_msg" {
            if let Some(turn) = event_turn(payload) {
                current_turn = Some(turn);
            }
        }
        // A call's own attribution remains authoritative when turns interleave.
        let call_turn = nonempty(&payload["internal_chat_message_metadata_passthrough"]["turn_id"])
            .or(current_turn);
        if call_turn != Some(turn)
            || r["type"] != "response_item"
            || payload["type"] != "custom_tool_call"
            || !payload["input"]
                .as_str()
                .is_some_and(|input| input.contains(&quoted))
        {
            continue;
        }
        let Some(timestamp) = utc_timestamp(&r["timestamp"]) else {
            continue;
        };
        if request.recorded_at < timestamp {
            continue;
        }
        let id = nonempty(&payload["call_id"]).or_else(|| nonempty(&payload["id"]))?;
        if pending_only && rollout[i + 1..].iter().any(|r| call_output(r, id)) {
            continue;
        }
        request.request_id = id.into();
        candidates.push(request.clone());
    }
    Some(candidates)
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
    match_records(records, transcript, claude_candidates)
}

/// Correlate a session's spool records, retaining the newest metadata per call.
pub fn match_codex_records(records: &[Value], rollout: &[Value]) -> Vec<PendingRequest> {
    match_records(records, rollout, codex_candidates)
}

fn match_records(
    records: &[Value],
    history: &[Value],
    matcher: fn(&Value, &[Value], bool) -> Option<Vec<PendingRequest>>,
) -> Vec<PendingRequest> {
    let mut latest = std::collections::BTreeMap::<String, PendingRequest>::new();
    // Pair in recording order before discarding resolved calls. Timestamp
    // eligibility alone cannot distinguish parallel identical calls.
    let mut ordered: Vec<_> = records.iter().enumerate().collect();
    ordered.sort_by(|(ai, a), (bi, b)| {
        a["t"].as_f64().unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&b["t"].as_f64().unwrap_or(f64::NEG_INFINITY))
            .then(ai.cmp(bi))
    });
    let mut paired = std::collections::BTreeSet::new();
    let mut assignments = Vec::new();
    for (_, record) in ordered {
        let Some(candidates) = matcher(record, history, false) else {
            continue;
        };
        let Some(request) = candidates.iter()
            .find(|r| !paired.contains(&r.request_id))
            .or_else(|| candidates.last())
            .cloned()
        else {
            continue;
        };
        paired.insert(request.request_id.clone());
        let pending = matcher(record, history, true).unwrap_or_default();
        assignments.push((request, pending));
    }
    // All ordinal claims must be known before recovering a record whose
    // earlier identical call survived longer than its pruned spool record.
    for (mut request, pending) in assignments {
        if !pending.iter().any(|r| r.request_id == request.request_id) {
            let mut unclaimed = pending.into_iter()
                .filter(|r| !paired.contains(&r.request_id));
            let Some(fallback) = unclaimed.next() else {
                continue;
            };
            if unclaimed.next().is_some() {
                continue;
            }
            paired.insert(fallback.request_id.clone());
            request = fallback;
        }
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
