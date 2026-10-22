use super::*;
use serde_json::json;

fn rows(source: &str) -> Vec<Value> {
    source
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn claude() -> Vec<Value> {
    // Synthetic tool-use-only history for the retained legacy transcript matcher;
    // real live pending transcripts do not include this call.
    rows(include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-transcript.jsonl"
    ))
    .into_iter()
    .take_while(|r| {
        !r["message"]["content"]
            .as_array()
            .is_some_and(|bs| bs.iter().any(|b| b["type"] == "tool_result"))
    })
    .collect()
}
fn sync() -> Vec<Value> {
    rows(include_str!(
        "../../../compatibility/phase2/fixtures/codex-sync-rollout-pending.jsonl"
    ))
}
fn asynchronous() -> Vec<Value> {
    rows(include_str!(
        "../../../compatibility/phase2/fixtures/codex-async-rollout-pending.jsonl"
    ))
}

#[test]
fn pending_fixtures_have_one_request_and_preserve_shapes() {
    let found = pending_claude_questions(&claude());
    assert_eq!(found.len(), 1);
    let request = &found[0];
    assert_eq!(request.kind, QuestionKind::ClaudeAsk);
    assert_eq!(request.turn_id, None);
    assert_eq!(request.questions.len(), 2);
    let q = &request.questions[0];
    assert_eq!(q.id, None);
    assert_eq!(q.header.as_deref(), Some("Size"));
    assert_eq!(q.text, "Pick a size");
    assert!(!q.multi_select);
    assert_eq!(
        q.options[0],
        QuestionOption {
            label: "Small".into(),
            description: Some("Choose small".into())
        }
    );
    assert!(request.questions[1].multi_select);
    assert_eq!(request.questions[1].options[2].label, "Basil");
    let history = claude();
    let use_id = history
        .iter()
        .flat_map(|r| r["message"]["content"].as_array().into_iter().flatten())
        .find(|b| b["name"] == "AskUserQuestion")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    assert_eq!(request.request_id, use_id);

    let found = pending_codex_questions(&sync());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, QuestionKind::CodexPlan);
    assert!(found[0].turn_id.is_some());
    assert_eq!(found[0].questions.len(), 2);
    assert!(found[0].questions[0].id.is_some());
    assert_eq!(found[0].questions[0].header.as_deref(), Some("Size"));
    assert_eq!(
        found[0].questions[0].options[0].description.as_deref(),
        Some("Choose a small size.")
    );
    assert!(found[0].questions.iter().all(|q| !q.multi_select));

    let found = pending_codex_questions(&asynchronous());
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, QuestionKind::CodexAsync);
    assert!(found[0].turn_id.is_some());
    assert_eq!(found[0].questions.len(), 1);
    let q = &found[0].questions[0];
    assert_eq!(q.text, "Color: Pick a color");
    assert_eq!(q.header, None);
    assert_eq!(q.id, None);
    assert_eq!(
        q.options,
        vec![
            QuestionOption {
                label: "Red (Choose red)".into(),
                description: None
            },
            QuestionOption {
                label: "Blue (Choose blue)".into(),
                description: None
            },
        ]
    );
    for request in pending_claude_questions(&claude())
        .into_iter()
        .chain(pending_codex_questions(&sync()))
        .chain(found)
    {
        assert!(
            request
                .questions
                .iter()
                .all(|q| q.allows_other && !q.secret)
        );
    }
}

#[test]
fn answered_fixtures_have_no_pending_requests() {
    assert!(
        pending_claude_questions(&rows(include_str!(
            "../../../compatibility/phase2/fixtures/claude-question-transcript.jsonl"
        )))
        .is_empty()
    );
    for source in [
        include_str!("../../../compatibility/phase2/fixtures/codex-sync-rollout.jsonl"),
        include_str!("../../../compatibility/phase2/fixtures/codex-async-rollout.jsonl"),
    ] {
        assert!(pending_codex_questions(&rows(source)).is_empty());
    }
    // The reply itself resolves async, even before task_complete.
    let mut history = rows(include_str!(
        "../../../compatibility/phase2/fixtures/codex-async-rollout.jsonl"
    ));
    let index = history
        .iter()
        .position(|r| {
            r["payload"]["role"] == "user"
                && r.to_string().contains("<send_user_message_question_reply>")
        })
        .unwrap();
    history.truncate(index + 1);
    assert!(pending_codex_questions(&history).is_empty());
}

#[test]
fn codex_turn_end_is_scoped_to_its_turn_for_both_kinds() {
    for event in ["task_complete", "turn_aborted"] {
        let mut history = asynchronous();
        let turn = pending_codex_questions(&history)[0]
            .turn_id
            .clone()
            .unwrap();
        history.push(json!({"type":"event_msg","payload":{"type":event,"turn_id":"another-turn"}}));
        assert_eq!(pending_codex_questions(&history).len(), 1);
        history.push(json!({"type":"event_msg","payload":{"type":event,"turn_id":turn}}));
        assert!(pending_codex_questions(&history).is_empty());
        let mut history = sync();
        let turn = pending_codex_questions(&history)[0]
            .turn_id
            .clone()
            .unwrap();
        history.push(json!({"type":"event_msg","payload":{"type":event,"turn_id":turn}}));
        assert!(pending_codex_questions(&history).is_empty());
    }
}

fn async_call(id: &str) -> Value {
    json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input_async","call_id":id,
        "arguments":json!({"questions":[{"title":"A title: with a colon","options":["One","Two"]}]}).to_string()}})
}
fn reply(id: &str, index: Value, role: &str) -> Value {
    let link = json!(["request_user_input_async", id, index]).to_string();
    let body = json!([{"questionItemId":link,"answer":"One"}]).to_string();
    json!({"type":"response_item","payload":{"type":"message","role":role,"content":[{"type":"input_text",
        "text":format!("<send_user_message_question_reply>\n{body}\n</send_user_message_question_reply>")}]}})
}

#[test]
fn returns_all_pending_and_only_user_replies_with_valid_links_resolve_async() {
    let mut history = vec![async_call("a"), async_call("b")];
    for r in [
        reply("unrelated", json!(0), "user"),
        reply("a", json!(9), "user"),
        reply("a", json!(0), "assistant"),
        reply("a", json!(-1), "user"),
    ] {
        history.push(r);
        assert_eq!(pending_codex_questions(&history).len(), 2);
    }
    history.push(reply("a", json!(0), "user"));
    let found = pending_codex_questions(&history);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].request_id, "b");
    history.push(reply("b", json!(0), "user"));
    assert!(pending_codex_questions(&history).is_empty());

    let mut history = claude();
    let mut second = history
        .iter()
        .find(|r| r["type"] == "assistant")
        .unwrap()
        .clone();
    second["message"]["content"][0]["id"] = json!("second");
    history.push(second);
    assert_eq!(pending_claude_questions(&history).len(), 2);
}

#[test]
fn sync_secret_and_optional_flags_and_async_free_text() {
    let mut history = sync();
    let record = history
        .iter_mut()
        .find(|r| r["payload"]["name"] == "request_user_input")
        .unwrap();
    let mut args: Value =
        serde_json::from_str(record["payload"]["arguments"].as_str().unwrap()).unwrap();
    args["questions"][0]["isSecret"] = json!(true);
    args["questions"][0]["isOther"] = json!(false);
    record["payload"]["arguments"] = json!(args.to_string());
    let found = pending_codex_questions(&history);
    assert_eq!(found.len(), 1);
    assert!(found[0].questions[0].secret);
    assert!(found[0].questions[0].allows_other);
    assert!(!found[0].questions[1].secret);
    let mut call = async_call("text");
    call["payload"]["arguments"] = json!(json!({"questions":[{"title":"Free text"}]}).to_string());
    let found = pending_codex_questions(&[call]);
    assert_eq!(found.len(), 1);
    assert!(found[0].questions[0].options.is_empty());
}

#[test]
fn malformed_records_are_skipped_without_hiding_valid_requests() {
    let junk = vec![
        Value::Null,
        json!(42),
        json!({"type":"response_item","payload":null}),
        json!({"type":"assistant","message":{"content":"invalid"}}),
    ];
    assert!(pending_claude_questions(&junk).is_empty());
    assert!(pending_codex_questions(&junk).is_empty());
    for arguments in [
        "not json".to_string(),
        "{}".into(),
        json!({"questions":[]}).to_string(),
        json!({"questions":[{"title":3,"options":[]}]}).to_string(),
        json!({"questions":[{"title":"test","options":[{"label":"bad"}]}]}).to_string(),
    ] {
        let mut bad = async_call("bad");
        bad["payload"]["arguments"] = json!(arguments);
        assert!(pending_codex_questions(&[bad.clone()]).is_empty());
        assert_eq!(
            pending_codex_questions(&[bad, async_call("valid")]).len(),
            1
        );
    }
    let mut bad = async_call("bad");
    bad["payload"]["call_id"] = Value::Null;
    assert!(pending_codex_questions(&[bad]).is_empty());
    let mut history = claude();
    let bad = history
        .iter_mut()
        .find(|r| r["type"] == "assistant")
        .unwrap();
    bad["message"]["content"][0]["input"]["questions"][0]["multiSelect"] = json!("true");
    assert!(pending_claude_questions(&history).is_empty());
    let mut history = vec![async_call("a")];
    history.extend(junk);
    history.push(json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"text":"<send_user_message_question_reply>bad json</send_user_message_question_reply>"}]}}));
    assert_eq!(pending_codex_questions(&history).len(), 1);
}

#[test]
fn async_waits_for_every_question_index() {
    let mut call = async_call("many");
    call["payload"]["arguments"] = json!(
        json!({"questions":[
            {"title":"First", "options":["A","B"]},
            {"title":"Second", "options":["C","D"]}
        ]})
        .to_string()
    );
    let mut history = vec![call, reply("many", json!(0), "user")];
    assert_eq!(pending_codex_questions(&history).len(), 1);
    history.push(reply("many", json!(0), "user"));
    history.push(reply("many", json!(2), "user"));
    assert_eq!(pending_codex_questions(&history).len(), 1);
    history.push(reply("many", json!(1), "user"));
    assert!(pending_codex_questions(&history).is_empty());
}

#[test]
fn claude_sidechain_and_meta_calls_and_results_are_ignored() {
    let call = claude()
        .into_iter()
        .find(|r| r["type"] == "assistant")
        .unwrap();
    let id = call["message"]["content"][0]["id"].clone();
    let result = json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":"answer"}]}});
    for flag in ["isSidechain", "isMeta"] {
        let mut excluded_call = call.clone();
        excluded_call[flag] = json!(true);
        assert!(pending_claude_questions(&[excluded_call]).is_empty());
        let mut excluded_result = result.clone();
        excluded_result[flag] = json!(true);
        let history = vec![call.clone(), excluded_result];
        assert_eq!(pending_claude_questions(&history).len(), 1);
        let mut resolved = history;
        resolved.push(result.clone());
        assert!(pending_claude_questions(&resolved).is_empty());
        let mut main = call.clone();
        main[flag] = json!(false);
        assert_eq!(pending_claude_questions(&[main]).len(), 1);
    }
}

fn spool_question(t: f64) -> Value {
    let history = rows(include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-transcript.jsonl"
    ));
    let call = history
        .iter()
        .find(|r| {
            r["message"]["content"]
                .as_array()
                .is_some_and(|bs| bs.iter().any(|b| b["name"] == "AskUserQuestion"))
        })
        .unwrap();
    let input = &call["message"]["content"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "AskUserQuestion")
        .unwrap()["input"];
    json!({"v":1,"t":t,"agent":"claude","event":"PermissionRequest","session_id":call["sessionId"],"payload":{"session_id":call["sessionId"],"prompt_id":"prompt","tool_name":"AskUserQuestion","tool_input":input}})
}
#[test]
fn claude_spool_is_pending_before_the_call_is_written_and_resolves_after_answer_or_cancel() {
    let history = rows(include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-transcript.jsonl"
    ));
    let call = history
        .iter()
        .position(|r| {
            r["message"]["content"]
                .as_array()
                .is_some_and(|bs| bs.iter().any(|b| b["name"] == "AskUserQuestion"))
        })
        .unwrap();
    let record = spool_question(1791849601.328234);
    let found = pending_claude_spool_questions(std::slice::from_ref(&record), &history[..call]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].request_id, "ask-prompt-1791849601328");
    assert_eq!(found[0].questions[0].text, "Pick a size");
    assert!(pending_claude_spool_questions(std::slice::from_ref(&record), &history).is_empty());
    let mut cancelled = history;
    for r in &mut cancelled {
        if r["type"] == "user" {
            if let Some(blocks) = r["message"]["content"].as_array_mut() {
                for b in blocks {
                    if b["type"] == "tool_result" {
                        b["is_error"] = json!(true);
                        b["content"] = json!("User cancelled");
                    }
                }
            }
        }
    }
    assert!(pending_claude_spool_questions(&[record], &cancelled).is_empty());
}
#[test]
fn claude_spool_pairs_repeated_identical_inputs_one_to_one_by_time() {
    let history = rows(include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-transcript.jsonl"
    ));
    let records = [spool_question(1791849631.0), spool_question(1791849601.0)];
    let found = pending_claude_spool_questions(&records, &history);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].request_id, "ask-prompt-1791849631000");
    let mut two_answers = history.clone();
    two_answers.extend(answered_claude_at(Some("2026-10-13T00:00:30Z")));
    assert!(pending_claude_spool_questions(&records, &two_answers).is_empty());
}
#[test]
fn claude_spool_ignores_sidechain_meta_and_foreign_session_answers() {
    let history = rows(include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-transcript.jsonl"
    ));
    for field in ["isSidechain", "isMeta", "foreign"] {
        let mut excluded = history.clone();
        for record in &mut excluded {
            if field == "foreign" {
                record["sessionId"] = json!("other");
            } else {
                record[field] = json!(true);
            }
        }
        assert_eq!(
            pending_claude_spool_questions(&[spool_question(1791849601.0)], &excluded).len(),
            1,
            "{field}"
        );
        // Keep the main call visible: an excluded result alone cannot retire it.
        let mut excluded_result = history.clone();
        for record in &mut excluded_result {
            if record["type"] == "user"
                && record["message"]["content"]
                    .as_array()
                    .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "tool_result"))
            {
                if field == "foreign" {
                    record["sessionId"] = json!("other");
                } else {
                    record[field] = json!(true);
                }
            }
        }
        assert_eq!(
            pending_claude_spool_questions(&[spool_question(1791849601.0)], &excluded_result).len(),
            1
        );
    }
}
#[test]
fn claude_spool_rejects_truncated_malformed_and_unmatched_sessions() {
    for case in [
        "truncated",
        "payload_truncated",
        "prompt",
        "time",
        "negative",
        "session",
        "event",
        "tool",
        "input",
    ] {
        let mut record = spool_question(1791849601.0);
        match case {
            "truncated" => record["truncated"] = json!(true),
            "payload_truncated" => record["payload"]["truncated"] = json!(true),
            "prompt" => {
                record["payload"]
                    .as_object_mut()
                    .unwrap()
                    .remove("prompt_id");
            }
            "time" => record["t"] = json!("bad"),
            "negative" => record["t"] = json!(-1),
            "session" => record["payload"]["session_id"] = json!("other"),
            "event" => record["event"] = json!("Other"),
            "tool" => record["payload"]["tool_name"] = json!("Bash"),
            "input" => record["payload"]["tool_input"] = json!({}),
            _ => unreachable!(),
        }
        assert!(
            pending_claude_spool_questions(&[record], &[]).is_empty(),
            "{case}"
        );
    }
}

fn answered_claude_at(timestamp: Option<&str>) -> Vec<Value> {
    let mut history = rows(include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-transcript.jsonl"
    ));
    for record in &mut history {
        if record["type"] == "assistant" {
            if let Some(timestamp) = timestamp {
                record["timestamp"] = json!(timestamp);
            } else {
                record.as_object_mut().unwrap().remove("timestamp");
            }
        }
    }
    history
}
#[test]
fn claude_spool_old_cancelled_question_does_not_consume_new_identical_request() {
    let mut history = answered_claude_at(Some("2026-10-22T06:52:18Z"));
    for record in &mut history {
        if record["type"] == "user" {
            record["timestamp"] = json!("2026-10-22T07:03:31Z");
            if let Some(blocks) = record["message"]["content"].as_array_mut() {
                for block in blocks {
                    if block["type"] == "tool_result" {
                        block["is_error"] = json!(true);
                    }
                }
            }
        }
    }
    let record = spool_question(1792652638.0); // 2026-10-22 07:03:58 UTC
    let found = pending_claude_spool_questions(&[record], &history);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].request_id, "ask-prompt-1792652638000");
}
#[test]
fn claude_spool_recent_answer_resolves_only_the_first_record_thirty_seconds_apart() {
    let history = answered_claude_at(Some("2026-10-13T00:00:00Z"));
    let first = spool_question(1791849601.0);
    assert!(pending_claude_spool_questions(std::slice::from_ref(&first), &history).is_empty());
    let second = spool_question(1791849631.0);
    let found = pending_claude_spool_questions(&[second, first], &history);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].request_id, "ask-prompt-1791849631000");
}
#[test]
fn claude_spool_requires_a_valid_call_timestamp_within_the_inclusive_window() {
    let record = spool_question(1791849601.0);
    for (timestamp, resolves) in [
        (Some("2026-10-12T23:58:01Z"), true), // exactly 120s before record
        (Some("2026-10-12T23:58:00.999Z"), false),
        (Some("2026-10-13T00:00:03Z"), true), // exactly 2s after record
        (Some("2026-10-13T00:00:03.001Z"), false),
        (None, false),
        (Some("invalid"), false),
    ] {
        let history = answered_claude_at(timestamp);
        assert_eq!(
            pending_claude_spool_questions(std::slice::from_ref(&record), &history).is_empty(),
            resolves,
            "{timestamp:?}"
        );
    }
}
#[test]
fn claude_spool_ignores_out_of_window_answers_and_pairs_calls_in_timestamp_order() {
    let old = answered_claude_at(Some("2026-10-12T23:00:00Z"));
    let earlier = answered_claude_at(Some("2026-10-12T23:58:20Z"));
    let recent = answered_claude_at(Some("2026-10-13T00:00:00Z"));
    let mut history = recent;
    history.extend(old);
    history.extend(earlier);
    let records = [spool_question(1791849631.0), spool_question(1791849601.0)];
    assert!(pending_claude_spool_questions(&records, &history).is_empty());
}
