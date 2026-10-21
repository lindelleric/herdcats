use herdcats_bridge::approval_match::{
    Resolution, claude_resolution, codex_resolution, match_claude, match_claude_records,
    match_codex, match_codex_records,
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
    json!({"type":"assistant","timestamp":"1970-01-01T00:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","id":id,"name":h["payload"]["tool_name"],"input":h["payload"]["tool_input"]}]}})
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
    let mut other_turn = call.clone();
    other_turn["payload"]["internal_chat_message_metadata_passthrough"]["turn_id"] = json!("other");
    p.push(other_turn);
    // A matching call attributed to another turn must not introduce ambiguity.
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
    // Embedded metadata suffices even without preceding turn-bearing events.
    assert!(match_codex(&h, &p[p.len() - 1..]).is_some());
    p.last_mut().unwrap()["payload"]
        .as_object_mut()
        .unwrap()
        .remove("internal_chat_message_metadata_passthrough");
    assert!(match_codex(&h, &p[p.len() - 1..]).is_none());
}

#[test]
fn codex_event_turns_fall_back_to_root_turn_id() {
    let h = hook("codex", "Bash");
    let turn = h["payload"]["turn_id"].as_str().unwrap();
    let mut p = pending();
    p.last_mut().unwrap()["payload"]
        .as_object_mut()
        .unwrap()
        .remove("internal_chat_message_metadata_passthrough");
    for record in &mut p {
        if record["type"] == "event_msg" {
            let payload = record["payload"].as_object_mut().unwrap();
            if let Some(id) = payload.remove("turn_id") {
                payload.insert("root_turn_id".into(), id);
            }
        }
    }
    assert!(match_codex(&h, &p).is_some());
    let abort = json!({"type":"event_msg","payload":{"type":"turn_aborted","root_turn_id":turn}});
    assert_eq!(
        codex_resolution("pending", turn, std::slice::from_ref(&abort)),
        Resolution::Aborted
    );
    assert_eq!(
        codex_resolution("pending", "other", std::slice::from_ref(&abort)),
        Resolution::Pending
    );
    let mut both = abort;
    both["payload"]["turn_id"] = json!("other");
    assert_eq!(
        codex_resolution("pending", turn, &[both]),
        Resolution::Pending
    );
    p.insert(
        p.len() - 1,
        json!({"type":"event_msg","payload":{"type":"item_completed","root_turn_id":"other"}}),
    );
    assert!(match_codex(&h, &p).is_none());
}

#[test]
fn claude_records_keep_latest_hook_for_repeated_pending_call() {
    let mut older = hook("claude", "Bash");
    older["t"] = json!(1.0);
    older["payload"]["permission_suggestions"] = json!(["old"]);
    let mut newer = older.clone();
    newer["t"] = json!(2.0);
    newer["payload"]["permission_suggestions"] = json!(["new"]);
    let transcript = [
        tool_use("completed", &older),
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"completed"}]}}),
        tool_use("pending", &newer),
    ];
    let matched = match_claude_records(&[newer.clone(), older], &transcript);
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].request_id, "pending");
    assert_eq!(matched[0].recorded_at, 2.0);
    assert_eq!(matched[0].suggestions, Some(json!(["new"])));
    let mut tied = newer.clone();
    tied["payload"]["permission_suggestions"] = json!(["tie wins"]);
    assert_eq!(
        match_claude_records(&[newer, tied], &transcript)[0].suggestions,
        Some(json!(["tie wins"]))
    );
}

#[test]
fn codex_records_keep_latest_hook_and_sort_by_recorded_at() {
    let mut older = hook("codex", "Bash");
    older["t"] = json!(1.0);
    let mut newer = older.clone();
    newer["t"] = json!(3.0);
    let mut other = older.clone();
    other["t"] = json!(2.0);
    other["payload"]["tool_input"]["command"] = json!("echo other");
    let mut rollout = pending();
    rollout.last_mut().unwrap()["timestamp"] = json!("1970-01-01T00:00:00Z");
    let mut call = rollout.last().unwrap().clone();
    call["payload"]["call_id"] = json!("other");
    call["payload"]["input"] = json!("exec({cmd:\"echo other\"})");
    rollout.push(call);
    let matched = match_codex_records(&[newer.clone(), other, older], &rollout);
    assert_eq!(matched.len(), 2);
    assert_eq!(matched[0].request_id, "other");
    assert_eq!(matched[0].recorded_at, 2.0);
    assert_eq!(
        matched[1].request_id,
        "a9716926-1865-55d2-9159-819c0fb40697"
    );
    assert_eq!(matched[1].recorded_at, 3.0);
    let mut tied = newer.clone();
    tied["payload"]["tool_input"]["description"] = json!("tie wins");
    assert_eq!(
        match_codex_records(&[newer, tied], &rollout)[0]
            .justification
            .as_deref(),
        Some("tie wins")
    );
    assert!(match_claude_records(&[], &[]).is_empty());
    assert!(match_codex_records(&[], &[]).is_empty());
}

#[test]
fn codex_embedded_turn_overrides_interleaved_event_position() {
    let h = hook("codex", "Bash");
    let mut rollout = pending();
    rollout.insert(
        rollout.len() - 1,
        json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"other"}}),
    );
    assert_eq!(
        match_codex(&h, &rollout).unwrap().request_id,
        "a9716926-1865-55d2-9159-819c0fb40697"
    );

    let mut rollout = pending();
    rollout.last_mut().unwrap()["payload"]["internal_chat_message_metadata_passthrough"]["turn_id"] =
        json!("other");
    assert!(match_codex(&h, &rollout).is_none());
}

#[test]
fn codex_calls_without_embedded_turn_use_position() {
    let h = hook("codex", "Bash");
    for metadata in [Value::Null, json!({}), json!({"turn_id":""})] {
        let mut rollout = pending();
        rollout.last_mut().unwrap()["payload"]["internal_chat_message_metadata_passthrough"] =
            metadata;
        assert!(match_codex(&h, &rollout).is_some());
        rollout.insert(
            rollout.len() - 1,
            json!({"type":"event_msg","payload":{"type":"item_completed","turn_id":"other"}}),
        );
        assert!(match_codex(&h, &rollout).is_none());
    }
}

#[test]
fn claude_stale_hook_does_not_attach_to_repeated_pending_call() {
    let mut h = hook("claude", "Bash");
    h["t"] = json!(1.0);
    let mut newer_call = tool_use("pending", &h);
    newer_call["timestamp"] = json!("1970-01-01T00:00:10Z");
    let transcript = [
        tool_use("old", &h),
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"old"}]}}),
        newer_call,
    ];
    assert!(match_claude(&h, &transcript).is_none());
    assert!(match_claude_records(std::slice::from_ref(&h), &transcript).is_empty());
    h["t"] = json!(10.0);
    assert_eq!(match_claude(&h, &transcript).unwrap().request_id, "pending");
}

#[test]
fn claude_timestamp_fraction_requires_hook_at_or_after_call() {
    let mut h = hook("claude", "Bash");
    let mut call = tool_use("pending", &h);
    call["timestamp"] = json!("1970-01-01T00:00:01.123Z");
    for t in [0.723, 1.122] {
        h["t"] = json!(t);
        assert!(match_claude(&h, std::slice::from_ref(&call)).is_none());
    }
    for t in [1.123, 1.124] {
        h["t"] = json!(t);
        assert!(match_claude(&h, std::slice::from_ref(&call)).is_some());
    }
    h["t"] = json!(2.0);
    for timestamp in [
        Value::Null,
        json!("bad"),
        json!("1970-01-01T00:00:01.Z"),
        json!("1970-01-01T00:00:01.1xZ"),
        json!("1970-01-01T24:00:00Z"),
        json!("1970-01-01T00:60:00Z"),
        json!("2026-02-29T00:00:00Z"),
        json!("2026-13-01T00:00:00Z"),
        json!("2026-01-00T00:00:00Z"),
        json!("é970-01-01T00:00:00Z"),
    ] {
        call["timestamp"] = timestamp;
        assert!(match_claude(&h, std::slice::from_ref(&call)).is_none());
    }
    call.as_object_mut().unwrap().remove("timestamp");
    assert!(match_claude(&h, &[call]).is_none());
}

#[test]
fn claude_timestamp_calendar_conversion() {
    let mut h = hook("claude", "Bash");
    for (timestamp, seconds) in [
        ("2000-02-29T00:00:00Z", 951782400.0),
        ("1900-03-01T00:00:00Z", -2203891200.0),
        ("1970-01-02T01:02:03.25Z", 90123.25),
    ] {
        let mut call = tool_use("pending", &h);
        call["timestamp"] = json!(timestamp);
        h["t"] = json!(seconds);
        assert!(
            match_claude(&h, std::slice::from_ref(&call)).is_some(),
            "{timestamp}"
        );
        h["t"] = json!(seconds - 0.001);
        assert!(match_claude(&h, &[call]).is_none(), "{timestamp}");
    }
}

#[test]
fn codex_call_without_call_id_uses_id_but_outputs_require_call_id() {
    let h = hook("codex", "Bash");
    let mut rollout = pending();
    let payload = rollout.last_mut().unwrap()["payload"]
        .as_object_mut()
        .unwrap();
    payload.remove("call_id");
    let id = payload["id"].as_str().unwrap().to_string();
    assert_eq!(match_codex(&h, &rollout).unwrap().request_id, id);
    assert_eq!(codex_resolution(&id, "turn", &rollout), Resolution::Pending);
    let mut id_only_output = output("custom_tool_call_output", &id);
    id_only_output["payload"]
        .as_object_mut()
        .unwrap()
        .remove("call_id");
    id_only_output["payload"]["id"] = json!(id);
    rollout.push(id_only_output);
    assert!(match_codex(&h, &rollout).is_some());
    assert_eq!(codex_resolution(&id, "turn", &rollout), Resolution::Pending);
    for kind in ["custom_tool_call_output", "function_call_output"] {
        let mut resolved = rollout.clone();
        resolved.push(output(kind, &id));
        assert!(match_codex(&h, &resolved).is_none());
        assert_eq!(
            codex_resolution(&id, "turn", &resolved),
            Resolution::Resolved
        );
    }
}

#[test]
fn codex_stale_hook_does_not_attach_to_repeated_pending_call() {
    let mut old = hook("codex", "Bash");
    old["t"] = json!(1.0);
    let mut rollout = pending();
    let first = rollout.last_mut().unwrap();
    first["timestamp"] = json!("1970-01-01T00:00:00Z");
    first["payload"]["call_id"] = json!("completed");
    let mut second = first.clone();
    second["timestamp"] = json!("1970-01-01T00:00:10.123Z");
    second["payload"]["call_id"] = json!("pending");
    rollout.push(output("custom_tool_call_output", "completed"));
    rollout.push(second);
    assert!(match_codex(&old, &rollout).is_none());
    assert!(match_codex_records(std::slice::from_ref(&old), &rollout).is_empty());
    let mut new = old.clone();
    new["t"] = json!(11.0);
    new["payload"]["tool_input"]["description"] = json!("new hook");
    let matched = match_codex_records(&[new, old], &rollout);
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].request_id, "pending");
    assert_eq!(matched[0].recorded_at, 11.0);
    assert_eq!(matched[0].justification.as_deref(), Some("new hook"));
}

#[test]
fn codex_timestamp_requires_valid_utc_and_hook_at_or_after_call() {
    let mut h = hook("codex", "Bash");
    let mut rollout = pending();
    rollout.last_mut().unwrap()["timestamp"] = json!("1970-01-01T00:00:01.123Z");
    for t in [0.723, 1.122] {
        h["t"] = json!(t);
        assert!(match_codex(&h, &rollout).is_none());
    }
    for t in [1.123, 1.124] {
        h["t"] = json!(t);
        assert!(match_codex(&h, &rollout).is_some());
    }
    h["t"] = json!(2.0);
    for timestamp in [
        Value::Null,
        json!("bad"),
        json!("1970-01-01T00:00:01.Z"),
        json!("2026-02-29T00:00:00Z"),
    ] {
        rollout.last_mut().unwrap()["timestamp"] = timestamp;
        assert!(match_codex(&h, &rollout).is_none());
    }
    rollout
        .last_mut()
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove("timestamp");
    assert!(match_codex(&h, &rollout).is_none());
}

#[test]
fn identical_parallel_calls_bind_hooks_before_resolution() {
    for agent in ["claude", "codex"] {
        let mut first = hook(agent, "Bash");
        first["t"] = json!(2000000000.0);
        first["payload"]["permission_suggestions"] = json!(["first"]);
        let mut second = first.clone();
        second["t"] = json!(2000000001.0);
        second["payload"]["permission_suggestions"] = json!(["second"]);
        if agent == "claude" {
            let mut message = tool_use("first", &first);
            let b = tool_use("second", &first)["message"]["content"][0].clone();
            message["message"]["content"].as_array_mut().unwrap().push(b);
            assert_eq!(match_claude_records(&[first.clone(), second.clone()], &[message.clone()]).len(), 2);
            let history = [message, json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"first"}]}})];
            let matched = match_claude_records(&[first, second], &history);
            assert_eq!(matched.len(), 1);
            assert_eq!(matched[0].request_id, "second");
            assert_eq!(matched[0].suggestions, Some(json!(["second"])));
        } else {
            let mut history = pending();
            history.last_mut().unwrap()["payload"]["call_id"] = json!("first");
            let mut call = history.last().unwrap().clone();
            call["payload"]["call_id"] = json!("second");
            history.push(call);
            assert_eq!(match_codex_records(&[first.clone(), second.clone()], &history).len(), 2);
            history.push(output("custom_tool_call_output", "first"));
            let matched = match_codex_records(&[first, second], &history);
            assert_eq!(matched.len(), 1);
            assert_eq!(matched[0].request_id, "second");
            assert_eq!(matched[0].recorded_at, 2000000001.0);
        }
    }
}


#[test]
fn claude_pruned_hook_falls_back_only_to_one_unclaimed_pending_call() {
    let mut h = hook("claude", "Bash");
    h["payload"]["permission_suggestions"] = json!(["retained"]);
    let mut history = vec![
        tool_use("resolved", &h),
        tool_use("pending", &h),
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"resolved"}]}}),
    ];
    let matched = match_claude_records(std::slice::from_ref(&h), &history);
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].request_id, "pending");
    assert_eq!(matched[0].suggestions, Some(json!(["retained"])));
    history.push(tool_use("also_pending", &h));
    assert!(match_claude_records(&[h], &history).is_empty());
}

#[test]
fn codex_pruned_hook_falls_back_only_to_one_unclaimed_pending_call() {
    let mut h = hook("codex", "Bash");
    h["payload"]["tool_input"]["description"] = json!("retained");
    let mut history = pending();
    history.last_mut().unwrap()["payload"]["call_id"] = json!("resolved");
    let mut call = history.last().unwrap().clone();
    call["payload"]["call_id"] = json!("pending");
    history.push(call.clone());
    history.push(output("custom_tool_call_output", "resolved"));
    let matched = match_codex_records(std::slice::from_ref(&h), &history);
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].request_id, "pending");
    assert_eq!(matched[0].justification.as_deref(), Some("retained"));
    call["payload"]["call_id"] = json!("also_pending");
    history.push(call);
    assert!(match_codex_records(&[h], &history).is_empty());
}
