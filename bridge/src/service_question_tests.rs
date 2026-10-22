// Included in service::tests to exercise the real polling path through fake Herdr.
fn question_fixture(kind: &str) -> (Service, PathBuf, SharedHerdr, PathBuf) {
    let (mut service, root, session, herdr) = fixture_with_herdr();
    service.include_questions = true;
    let (source, screen) = match kind {
        "claude" => ("claude-question-transcript", "claude-question-single"),
        "codex_plan" => ("codex-sync-rollout", "codex-sync-first"),
        "codex_async" => ("codex-async-rollout", "codex-async-queued"),
        _ => unreachable!(),
    };
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../compatibility/phase2/fixtures");
    let text = fs::read_to_string(fixtures.join(format!("{source}-pending.jsonl"))).unwrap();
    let records: Vec<Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let claude = kind == "claude";
    let native = if claude {
        records[0]["sessionId"].as_str().unwrap()
    } else {
        records[0]["payload"]["id"].as_str().unwrap()
    };
    *session.lock().unwrap() = native.into();
    let path = if claude {
        root.join(format!("projects/one/{native}.jsonl"))
    } else {
        fs::create_dir_all(root.join("codex/2026/10/13")).unwrap();
        service = service.with_codex_root(root.join("codex"));
        root.join(format!("codex/2026/10/13/rollout-fixture-{native}.jsonl"))
    };
    fs::write(&path, text).unwrap();
    {
        let mut state = herdr.lock().unwrap();
        state.agent = Some(if claude { "claude" } else { "codex" }.into());
        state.status = Some("blocked".into());
        state.screen = fs::read_to_string(fixtures.join(format!("{screen}-screen.txt"))).unwrap();
    }
    (service, root, herdr, path)
}

#[test]
fn questions_emit_each_kind_cache_and_resolve_on_answer() {
    for (kind, answered) in [
        (
            "claude",
            include_str!("../../compatibility/phase2/fixtures/claude-question-transcript.jsonl"),
        ),
        (
            "codex_plan",
            include_str!("../../compatibility/phase2/fixtures/codex-sync-rollout.jsonl"),
        ),
        (
            "codex_async",
            include_str!("../../compatibility/phase2/fixtures/codex-async-rollout.jsonl"),
        ),
    ] {
        let (mut service, root, herdr, path) = question_fixture(kind);
        let events = interactions(&mut service);
        assert_eq!(events.len(), 1, "{kind}: {events:?}");
        let data = &events[0]["data"];
        assert_eq!(events[0]["event"], "interaction.pending");
        assert_eq!(data["kind"], "question");
        assert_eq!(data["question_kind"], kind);
        assert_eq!(data["fresh"], true);
        assert_eq!(data["questions"][0]["index"], 0);
        assert_eq!(data["questions"][0]["options"][0]["number"], 1);
        let stamp = service
            .watch
            .as_ref()
            .unwrap()
            .question_cache
            .as_ref()
            .unwrap()
            .stamp
            .clone();
        let reads = herdr.lock().unwrap().reads;
        assert!(interactions(&mut service).is_empty());
        assert_eq!(herdr.lock().unwrap().reads, reads + 1);
        assert_eq!(
            service
                .watch
                .as_ref()
                .unwrap()
                .question_cache
                .as_ref()
                .unwrap()
                .stamp,
            stamp
        );
        fs::write(path, answered).unwrap();
        let resolved = interactions(&mut service);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0]["event"], "interaction.resolved");
        assert_eq!(resolved[0]["data"]["request_id"], data["request_id"]);
        assert!(interactions(&mut service).is_empty());
        assert!(herdr.lock().unwrap().keys.is_empty());
        cleanup(&service, root);
    }
}

#[test]
fn questions_require_independent_opt_in() {
    let (mut service, root, herdr, _) = question_fixture("claude");
    let mut out = Vec::new();
    service
        .request(
            br#"{"id":"i","method":"initialize","params":{"protocol":1,"approvals":true}}"#,
            &mut out,
        )
        .unwrap();
    assert_eq!(frames(out)[0]["result"]["capabilities"]["questions"], true);
    service
        .request(
            br#"{"id":"w","method":"watch","params":{"pane_id":"w1:p1"}}"#,
            &mut Vec::new(),
        )
        .unwrap();
    assert!(interactions(&mut service).is_empty());
    assert_eq!(herdr.lock().unwrap().reads, 0);
    let mut out = Vec::new();
    service.request(br#"{"id":"r","method":"interaction.respond","params":{"request_id":"q","answers":[]}}"#, &mut out).unwrap();
    assert_eq!(frames(out)[0]["error"]["code"], "unsupported_capability");
    service
        .request(
            br#"{"id":"i","method":"initialize","params":{"protocol":1,"questions":true}}"#,
            &mut Vec::new(),
        )
        .unwrap();
    let mut out = Vec::new();
    service
        .request(
            br#"{"id":"w","method":"watch","params":{"pane_id":"w1:p1"}}"#,
            &mut out,
        )
        .unwrap();
    assert!(frames(out).iter().any(|f| f["data"]["kind"] == "question"));
    assert!(!service.include_approvals);
    cleanup(&service, root);
}

#[test]
fn questions_suppress_secrets_and_multi_question_async() {
    for kind in ["codex_plan", "codex_async"] {
        let (mut service, root, herdr, path) = question_fixture(kind);
        let mut records: Vec<Value> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let name = if kind == "codex_plan" {
            "request_user_input"
        } else {
            "request_user_input_async"
        };
        let call = records
            .iter_mut()
            .find(|r| r["payload"]["name"] == name)
            .unwrap();
        let mut args: Value =
            serde_json::from_str(call["payload"]["arguments"].as_str().unwrap()).unwrap();
        if kind == "codex_plan" {
            args["questions"][0]["isSecret"] = json!(true);
        } else {
            let extra = args["questions"][0].clone();
            args["questions"].as_array_mut().unwrap().push(extra);
        }
        call["payload"]["arguments"] = json!(args.to_string());
        write_records(&path, &records);
        assert!(interactions(&mut service).is_empty());
        assert_eq!(herdr.lock().unwrap().reads, 0);
        cleanup(&service, root);
    }
}

#[test]
fn question_step_and_fresh_changes_emit_and_loss_of_screen_resolves() {
    let (mut service, root, herdr, _) = question_fixture("claude");
    assert_eq!(interactions(&mut service)[0]["data"]["fresh"], true);
    herdr.lock().unwrap().screen =
        include_str!("../../compatibility/phase2/fixtures/claude-question-multi-screen.txt").into();
    let events = interactions(&mut service);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["fresh"], false);
    assert_eq!(
        events[0]["data"]["step"],
        json!({"kind":"claude_question","index":1})
    );
    herdr.lock().unwrap().screen.clear();
    assert_eq!(
        interactions(&mut service)[0]["event"],
        "interaction.resolved"
    );
    cleanup(&service, root);
}

#[test]
fn approval_has_priority_over_pending_question() {
    let (mut service, root, herdr, _, path) = approval_fixture("claude");
    service.include_questions = true;
    let pending: Vec<Value> = include_str!(
        "../../compatibility/phase2/fixtures/claude-question-transcript-pending.jsonl"
    )
    .lines()
    .map(|l| serde_json::from_str(l).unwrap())
    .collect();
    let call = pending
        .iter()
        .find(|r| {
            r["message"]["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|b| b["name"] == "AskUserQuestion"))
        })
        .unwrap();
    let mut call = call.clone();
    call["sessionId"] = json!(
        serde_json::from_str::<Value>(fs::read_to_string(&path).unwrap().lines().next().unwrap())
            .unwrap()["sessionId"]
    );
    writeln!(
        fs::OpenOptions::new().append(true).open(&path).unwrap(),
        "{call}"
    )
    .unwrap();
    let events = interactions(&mut service);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["kind"], "approval");
    herdr.lock().unwrap().screen =
        include_str!("../../compatibility/phase2/fixtures/claude-question-single-screen.txt")
            .into();
    let events = interactions(&mut service);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["event"], "interaction.resolved");
    assert_eq!(events[1]["data"]["kind"], "question");
    cleanup(&service, root);
}

#[test]
fn question_freshness_changes_without_changing_step() {
    for kind in ["claude", "codex_plan", "codex_async"] {
        let (mut service, root, herdr, _) = question_fixture(kind);
        if kind == "codex_async" {
            herdr.lock().unwrap().screen =
                include_str!("../../compatibility/phase2/fixtures/codex-async-picker-screen.txt")
                    .into();
        }
        let first = interactions(&mut service).remove(0);
        assert_eq!(first["data"]["fresh"], true);
        {
            let mut state = herdr.lock().unwrap();
            state.screen = match kind {
                "claude" => state.screen.replacen("☐ Toppings", "☒ Toppings", 1),
                "codex_plan" => state.screen.replace("(2 unanswered)", "(1 unanswered)"),
                "codex_async" => state.screen.replace("› 1.", "  1.").replace("  2.", "› 2."),
                _ => unreachable!(),
            };
        }
        let changed = interactions(&mut service);
        assert_eq!(changed.len(), 1, "{kind}: {changed:?}");
        assert_eq!(changed[0]["event"], "interaction.pending");
        assert_eq!(changed[0]["data"]["fresh"], false);
        assert_eq!(changed[0]["data"]["step"], first["data"]["step"]);
        cleanup(&service, root);
    }
}

#[test]
fn questions_resolve_on_unblocked_or_replaced_identity_and_preserve_transient_reads() {
    for replacement in [false, true] {
        let (mut service, root, herdr, _) = question_fixture("claude");
        let first = interactions(&mut service).remove(0);
        let mut out = Vec::new();
        service
            .poll_with_refresh(&mut out, |_, _, _| Err("source_changing"))
            .unwrap();
        assert!(out.is_empty());
        assert!(interactions(&mut service).is_empty());
        {
            let mut state = herdr.lock().unwrap();
            if replacement {
                state.terminal_id = Some("replacement".into());
            } else {
                state.status = Some("idle".into());
            }
        }
        let events = interactions(&mut service);
        assert_eq!(events[0]["event"], "interaction.resolved");
        assert_eq!(events[0]["data"]["identity"], first["data"]["identity"]);
        if replacement {
            assert_eq!(events[1]["event"], "interaction.pending");
            assert_eq!(events[1]["data"]["identity"]["terminal_id"], "replacement");
        } else {
            assert!(service.watch.as_ref().unwrap().question_cache.is_none());
        }
        cleanup(&service, root);
    }
}
