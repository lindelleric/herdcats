use herdcats_bridge::approval_match::{
    Resolution, claude_resolution, codex_resolution, match_claude, match_codex,
};
use herdcats_bridge::hooks::Agent;
use serde_json::{Value, json};

fn records(text: &str) -> Vec<Value> {
    text.lines()
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}
fn hook(agent: &str, tool: &str) -> Value {
    let text = if agent == "claude" {
        include_str!("../../compatibility/phase1/fixtures/claude-hooks.jsonl")
    } else {
        include_str!("../../compatibility/phase1/fixtures/codex-hooks.jsonl")
    };
    let h = records(text)
        .into_iter()
        .find(|v| v["event"] == "PermissionRequest" && v["payload"]["tool_name"] == tool)
        .unwrap();
    json!({"v":1,"t":h["t"],"agent":agent,"event":"PermissionRequest","session_id":h["payload"]["session_id"],"pane":"w1:p1","payload":h["payload"]})
}
fn transcript() -> Vec<Value> {
    records(include_str!(
        "../../compatibility/phase1/fixtures/claude-transcript.jsonl"
    ))
}
fn pending() -> Vec<Value> {
    records(include_str!(
        "../../compatibility/phase1/fixtures/codex-pending-rollout.jsonl"
    ))
}
fn tool_use(id: &str, h: &Value) -> Value {
    json!({"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":id,"name":h["payload"]["tool_name"],"input":h["payload"]["tool_input"]}]}})
}
fn output(kind: &str, id: &str) -> Value {
    json!({"type":"response_item","payload":{"type":kind,"call_id":id,"output":"done"}})
}

#[test]
fn claude_fixtures_match_only_pending_requests() {
    let all = transcript();
    for (tool, id) in [
        ("Bash", "7ea4f26e-9291-5569-be48-6e0571d23ef8"),
        ("AskUserQuestion", "cc197b77-c47d-59a2-9bbe-a3b9e65d962a"),
    ] {
        let h = hook("claude", tool);
        let end = all
            .iter()
            .position(|v| {
                v["message"]["content"].as_array().is_some_and(|blocks| {
                    blocks
                        .iter()
                        .any(|b| b["type"] == "tool_result" && b["tool_use_id"] == id)
                })
            })
            .unwrap();
        let request = match_claude(&h, &all[..end]).unwrap();
        assert_eq!(request.agent, Agent::Claude);
        assert_eq!(request.request_id, id);
        assert_eq!(request.session_id, h["session_id"]);
        assert_eq!(request.tool_name, tool);
        assert_eq!(request.tool_input, h["payload"]["tool_input"]);
        assert_eq!(
            request.suggestions,
            h["payload"].get("permission_suggestions").cloned()
        );
        assert_eq!(request.justification, None);
        assert_eq!(request.recorded_at, h["t"].as_f64().unwrap());
        assert_eq!(claude_resolution(id, &all[..end]), Resolution::Pending);
        assert!(match_claude(&h, &all).is_none());
        assert_eq!(claude_resolution(id, &all), Resolution::Resolved);
    }
}

#[test]
fn claude_rejects_ambiguity_different_inputs_and_non_assistant_calls() {
    let mut h = hook("claude", "Bash");
    let a = tool_use("a", &h);
    assert!(match_claude(&h, &[a.clone(), tool_use("b", &h)]).is_none());
    h["payload"]["tool_input"]["command"] = json!("different");
    assert!(match_claude(&h, std::slice::from_ref(&a)).is_none());
    let mut user = tool_use("user", &h);
    user["type"] = json!("user");
    user["message"]["role"] = json!("user");
    assert!(match_claude(&h, &[user]).is_none());
    assert!(match_claude(&h, &[]).is_none());
}

#[test]
fn codex_fixture_matches_rollout_id_and_resolves() {
    let h = hook("codex", "Bash");
    let p = pending();
    let r = match_codex(&h, &p).unwrap();
    assert_eq!(r.request_id, "a9716926-1865-55d2-9159-819c0fb40697");
    assert_eq!(r.agent, Agent::Codex);
    assert_eq!(r.session_id, h["session_id"]);
    assert_eq!(r.tool_name, "Bash");
    assert_eq!(r.tool_input, h["payload"]["tool_input"]);
    assert_eq!(
        r.justification.as_deref(),
        h["payload"]["tool_input"]["description"].as_str()
    );
    assert_eq!(r.suggestions, None);
    assert_eq!(r.recorded_at, h["t"].as_f64().unwrap());
    let turn = h["payload"]["turn_id"].as_str().unwrap();
    assert_eq!(
        codex_resolution(&r.request_id, turn, &p),
        Resolution::Pending
    );
    let all = records(include_str!(
        "../../compatibility/phase1/fixtures/codex-rollout.jsonl"
    ));
    assert!(match_codex(&h, &all).is_none());
    assert_eq!(
        codex_resolution(&r.request_id, turn, &all),
        Resolution::Resolved
    );
}

#[test]
fn codex_turns_ambiguity_and_outputs() {
    let h = hook("codex", "Bash");
    let mut p = pending();
    let call = p.last().unwrap().clone();
    p.push(json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"other"}}));
    p.push(call.clone());
    // A matching call in another turn must not introduce ambiguity.
    assert!(match_codex(&h, &p).is_some());
    assert!(match_codex(&h, &p[p.len() - 2..]).is_none());
    let mut p = pending();
    let mut second = call;
    second["payload"]["call_id"] = json!("second");
    p.push(second);
    assert!(match_codex(&h, &p).is_none());
    let id = "a9716926-1865-55d2-9159-819c0fb40697";
    for kind in ["custom_tool_call_output", "function_call_output"] {
        let mut p = pending();
        p.push(output(kind, id));
        assert!(match_codex(&h, &p).is_none());
        assert_eq!(codex_resolution(id, "turn", &p), Resolution::Resolved);
    }
    let abort = json!({"type":"event_msg","payload":{"type":"turn_aborted","turn_id":"turn"}});
    assert_eq!(
        codex_resolution(id, "turn", std::slice::from_ref(&abort)),
        Resolution::Aborted
    );
    assert_eq!(codex_resolution(id, "other", &[abort]), Resolution::Pending);
    assert!(match_codex(&h, &[]).is_none());
}

#[test]
fn codex_requires_quoted_escaped_command() {
    let mut h = hook("codex", "Bash");
    let mut p = pending();
    h["payload"]["tool_input"]["command"] = json!("echo a");
    p.last_mut().unwrap()["payload"]["input"] = json!("exec({cmd:\"echo ab\"})");
    assert!(match_codex(&h, &p).is_none());
    let command = "echo \"a\\b\"";
    h["payload"]["tool_input"]["command"] = json!(command);
    p.last_mut().unwrap()["payload"]["input"] = json!(format!(
        "exec({{cmd:{}}})",
        serde_json::to_string(command).unwrap()
    ));
    assert!(match_codex(&h, &p).is_some());
    h["payload"]["tool_input"]["command"] = json!(42);
    assert!(match_codex(&h, &p).is_none());
}

#[test]
fn invalid_spool_records_are_ignored() {
    for agent in ["claude", "codex"] {
        let h = hook(agent, "Bash");
        for (field, value) in [
            ("v", json!(2)),
            ("event", json!("PreToolUse")),
            ("agent", json!("other")),
            ("session_id", Value::Null),
            ("t", Value::Null),
            ("payload", Value::Null),
        ] {
            let mut invalid = h.clone();
            invalid[field] = value;
            assert!(match_claude(&invalid, &[tool_use("id", &h)]).is_none());
            assert!(match_codex(&invalid, &pending()).is_none());
        }
    }
}

#[test]
fn only_later_outputs_exclude_candidates() {
    let h = hook("claude", "Bash");
    let result =
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"id"}]}});
    let call = tool_use("id", &h);
    assert!(match_claude(&h, &[result.clone(), call.clone()]).is_some());
    assert!(match_claude(&h, &[call, result]).is_none());

    let h = hook("codex", "Bash");
    let mut p = pending();
    p.insert(
        0,
        output(
            "custom_tool_call_output",
            "a9716926-1865-55d2-9159-819c0fb40697",
        ),
    );
    assert!(match_codex(&h, &p).is_some());
    // Without a preceding turn-bearing event, the call cannot be attributed.
    assert!(match_codex(&h, &p[p.len() - 1..]).is_none());
}
