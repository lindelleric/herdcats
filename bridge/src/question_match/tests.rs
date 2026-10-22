use super::*;
use serde_json::json;

fn rows(source: &str) -> Vec<Value> {
    source
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}
fn claude() -> Vec<Value> {
    rows(include_str!(
        "../../../compatibility/phase2/fixtures/claude-question-transcript-pending.jsonl"
    ))
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
