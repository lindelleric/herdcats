// Included inside service::tests to share the fake Herdr and private service API.
fn approval_fixture(agent: &str) -> (Service, PathBuf, SharedHerdr, Value, PathBuf) {
    let (mut service, root, session, herdr) = fixture_with_herdr();
    service.include_approvals = true;
    let (hooks, source, screen) = if agent == "claude" {
        (
            include_str!("../../compatibility/phase1/fixtures/claude-hooks.jsonl"),
            include_str!("../../compatibility/phase1/fixtures/claude-transcript.jsonl"),
            include_str!("../../compatibility/phase1/fixtures/claude-approval-screen.txt"),
        )
    } else {
        (
            include_str!("../../compatibility/phase1/fixtures/codex-hooks.jsonl"),
            include_str!("../../compatibility/phase1/fixtures/codex-pending-rollout.jsonl"),
            include_str!("../../compatibility/phase1/fixtures/codex-approval-screen.txt"),
        )
    };
    let hook: Value = hooks
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|r| r["event"] == "PermissionRequest" && r["payload"]["tool_name"] == "Bash")
        .unwrap();
    let native = hook["payload"]["session_id"].as_str().unwrap();
    *session.lock().unwrap() = native.into();
    let mut records: Vec<Value> = source
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let path = if agent == "claude" {
        assert!(records.iter().all(|r| r["sessionId"] == native));
        let end = records
            .iter()
            .position(|r| {
                r["message"]["content"]
                    .as_array()
                    .is_some_and(|b| b.iter().any(|b| b["type"] == "tool_result"))
            })
            .unwrap();
        records.truncate(end);
        root.join(format!("projects/one/{native}.jsonl"))
    } else {
        assert_eq!(records[0]["payload"]["id"], native);
        fs::create_dir_all(root.join("codex/2026/10/13")).unwrap();
        service = service.with_codex_root(root.join("codex"));
        root.join(format!("codex/2026/10/13/rollout-fixture-{native}.jsonl"))
    };
    write_records(&path, &records);
    fs::create_dir(root.join("spool")).unwrap();
    service.spool_dir = Some(root.join("spool"));
    let record = json!({"v":1,"t":hook["t"],"agent":agent,"event":"PermissionRequest","session_id":native,"payload":hook["payload"]});
    write_records(
        &root.join(format!("spool/{agent}-{native}.jsonl")),
        &[record],
    );
    {
        let mut state = herdr.lock().unwrap();
        state.agent = Some(agent.into());
        state.status = Some("blocked".into());
        state.screen = screen.into();
    }
    (service, root, herdr, hook, path)
}
fn write_records(path: &Path, records: &[Value]) {
    fs::write(
        path,
        records.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
}
fn interactions(service: &mut Service) -> Vec<Value> {
    let mut out = Vec::new();
    service.poll(&mut out).unwrap();
    frames(out)
        .into_iter()
        .filter(|r| r["event"].as_str().unwrap().starts_with("interaction."))
        .collect()
}

#[test]
fn approval_pending_payload_is_exact_and_unchanged_poll_is_silent() {
    for agent in ["claude", "codex"] {
        let (mut service, root, herdr, hook, _) = approval_fixture(agent);
        let emitted = interactions(&mut service);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0]["event"], "interaction.pending");
        let screen = herdr.lock().unwrap().screen.clone();
        let dialog = crate::approval_screen::parse(
            crate::hooks::Agent::parse(agent).unwrap(),
            &screen,
            hook["payload"]["tool_input"]["command"].as_str().unwrap(),
        )
        .unwrap();
        let options: Vec<_> = dialog.options.iter().map(|o| json!({"number":o.number,"label_lines":o.label_lines,"key":match o.key { crate::approval_screen::OptionKey::Digit(n) => n.to_string(), crate::approval_screen::OptionKey::Letter(c) => c.to_string(), crate::approval_screen::OptionKey::Escape => "esc".into() }})).collect();
        let request_id = if agent == "claude" {
            "7ea4f26e-9291-5569-be48-6e0571d23ef8"
        } else {
            "a9716926-1865-55d2-9159-819c0fb40697"
        };
        assert_eq!(
            emitted[0]["data"],
            json!({
                "identity":{"pane_id":"w1:p1","terminal_id":"terminal-one","agent":agent,"native_session_id":hook["payload"]["session_id"]},
                "generation":service.generation,"kind":"approval","request_id":request_id,"agent":agent,"tool_name":"Bash",
                "command":hook["payload"]["tool_input"]["command"],
                "description":if agent == "claude" { hook["payload"]["tool_input"]["description"].clone() } else { Value::Null },
                "justification":if agent == "codex" { hook["payload"]["tool_input"]["description"].clone() } else { Value::Null },
                "environment":dialog.environment,"title":dialog.title,"options":options,"selected":dialog.selected
            })
        );
        assert!(interactions(&mut service).is_empty());
        assert!(herdr.lock().unwrap().keys.is_empty());
        cleanup(&service, root);
    }
}

#[test]
fn approval_resolves_once_on_output_or_abort() {
    for (agent, abort) in [("claude", false), ("codex", false), ("codex", true)] {
        let (mut service, root, _, hook, path) = approval_fixture(agent);
        let pending = interactions(&mut service).remove(0);
        let id = pending["data"]["request_id"].clone();
        let record = if agent == "claude" {
            json!({"type":"user","uuid":"result","sessionId":hook["payload"]["session_id"],"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":"done"}]}})
        } else if abort {
            json!({"type":"event_msg","payload":{"type":"turn_aborted","turn_id":hook["payload"]["turn_id"]}})
        } else {
            json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":id,"output":"done"}})
        };
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        writeln!(file, "{record}").unwrap();
        let emitted = interactions(&mut service);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0]["event"], "interaction.resolved");
        assert_eq!(
            emitted[0]["data"],
            json!({"identity":pending["data"]["identity"],"generation":pending["data"]["generation"],"request_id":id})
        );
        assert!(interactions(&mut service).is_empty());
        cleanup(&service, root);
    }
}

#[test]
fn approval_requires_blocked_and_one_matching_visible_dialog() {
    for agent in ["claude", "codex"] {
        let (mut service, root, herdr, _, _) = approval_fixture(agent);
        herdr.lock().unwrap().status = Some("idle".into());
        assert!(interactions(&mut service).is_empty());
        assert_eq!(herdr.lock().unwrap().reads, 0);
        herdr.lock().unwrap().status = Some("blocked".into());
        let screen = herdr.lock().unwrap().screen.clone();
        for invalid in [
            screen.replace("herdcats-probe", "other-command"),
            format!("{screen}\n{screen}"),
        ] {
            herdr.lock().unwrap().screen = invalid;
            assert!(interactions(&mut service).is_empty());
        }
        herdr.lock().unwrap().screen = screen;
        assert_eq!(interactions(&mut service).len(), 1);
        herdr.lock().unwrap().status = Some("idle".into());
        let reads = herdr.lock().unwrap().reads;
        assert_eq!(
            interactions(&mut service)[0]["event"],
            "interaction.resolved"
        );
        assert_eq!(herdr.lock().unwrap().reads, reads);
        assert!(interactions(&mut service).is_empty());
        cleanup(&service, root);
    }
}

fn test_pending(request_id: &str) -> PendingApproval {
    let agent = Agent::Claude;
    let command = "echo herdcats-probe > probe.txt";
    PendingApproval {
        identity: Identity {
            pane_id: "w1:p1".into(),
            terminal_id: "terminal-one".into(),
            agent: "claude".into(),
            native_session_id: SESSION.into(),
        },
        generation: 1,
        request: PendingRequest {
            agent,
            session_id: SESSION.into(),
            request_id: request_id.into(),
            tool_name: "Bash".into(),
            tool_input: json!({"command":command,"description":"verbatim hook description"}),
            justification: None,
            turn_id: None,
            suggestions: None,
            recorded_at: 1.0,
        },
        dialog: approval_screen::parse(
            agent,
            include_str!("../../compatibility/phase1/fixtures/claude-approval-screen.txt"),
            command,
        )
        .unwrap(),
    }
}
fn test_watch() -> Watch {
    Watch {
        pane_id: "w1:p1".into(),
        identity: None,
        follower: Follower::default(),
        generation: 0,
        previous: Vec::new(),
        unavailable: None,
        status: String::new(),
        unknown_records: 0,
        dropped_entries: 0,
        source_changing_count: 0,
        delayed: false,
        agent: None,
        unavailable_agent: None,
        last_daemon_status: None,
        pending: None,
        approval_cache: None,
        question_cache: None,
        source_pane: None,
        model_stamp: None,
        last_model: None,
        model_override: None,
    }
}

#[test]
fn approval_state_emits_only_changes_and_resolves_before_replacement() {
    let mut service = Service::with_resolvers(PathBuf::from("unused.sock"), None, None);
    let mut watch = test_watch();
    let mut out = Vec::new();
    service
        .update_pending(&mut out, &mut watch, Some(PendingInteraction::Approval(test_pending("one"))))
        .unwrap();
    let first = frames(std::mem::take(&mut out));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["event"], "interaction.pending");
    assert_eq!(first[0]["data"]["description"], "verbatim hook description");
    service
        .update_pending(&mut out, &mut watch, Some(PendingInteraction::Approval(test_pending("one"))))
        .unwrap();
    assert!(out.is_empty());
    let mut selected = test_pending("one");
    selected.dialog.selected = Some(1);
    service
        .update_pending(&mut out, &mut watch, Some(PendingInteraction::Approval(selected)))
        .unwrap();
    assert_eq!(frames(std::mem::take(&mut out))[0]["data"]["selected"], 1);
    let mut changed = test_pending("one");
    changed.dialog.selected = Some(1);
    changed.dialog.options[0].label_lines = vec!["Changed".into(), "second row".into()];
    service
        .update_pending(&mut out, &mut watch, Some(PendingInteraction::Approval(changed)))
        .unwrap();
    assert_eq!(
        frames(std::mem::take(&mut out))[0]["data"]["options"][0]["label_lines"],
        json!(["Changed", "second row"])
    );
    service
        .update_pending(&mut out, &mut watch, Some(PendingInteraction::Approval(test_pending("two"))))
        .unwrap();
    let replacement = frames(std::mem::take(&mut out));
    assert_eq!(replacement.len(), 2);
    assert_eq!(replacement[0]["event"], "interaction.resolved");
    assert_eq!(replacement[0]["data"]["request_id"], "one");
    assert_eq!(replacement[1]["event"], "interaction.pending");
    assert_eq!(replacement[1]["data"]["request_id"], "two");
    service.update_pending(&mut out, &mut watch, None).unwrap();
    assert_eq!(
        frames(std::mem::take(&mut out))[0]["event"],
        "interaction.resolved"
    );
    service.update_pending(&mut out, &mut watch, None).unwrap();
    assert!(out.is_empty());
}

#[test]
fn approval_state_uses_old_identity_on_resolution_and_rejects_oversized_frame() {
    let mut service = Service::with_resolvers(PathBuf::from("unused.sock"), None, None);
    let mut watch = test_watch();
    let mut out = Vec::new();
    service
        .update_pending(&mut out, &mut watch, Some(PendingInteraction::Approval(test_pending("one"))))
        .unwrap();
    out.clear();
    let mut replacement = test_pending("one");
    replacement.identity.native_session_id = OTHER.into();
    replacement.generation = 2;
    service
        .update_pending(&mut out, &mut watch, Some(PendingInteraction::Approval(replacement)))
        .unwrap();
    let emitted = frames(std::mem::take(&mut out));
    assert_eq!(emitted.len(), 2);
    assert_eq!(emitted[0]["data"]["identity"]["native_session_id"], SESSION);
    assert_eq!(emitted[0]["data"]["generation"], 1);
    assert_eq!(emitted[1]["data"]["identity"]["native_session_id"], OTHER);
    let mut large = test_pending("large");
    large.request.tool_input["description"] = json!("\"".repeat(MAX_RECORD / 2));
    service
        .update_pending(&mut out, &mut watch, Some(PendingInteraction::Approval(large)))
        .unwrap();
    let emitted = frames(std::mem::take(&mut out));
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0]["event"], "interaction.resolved");
    assert!(watch.pending.is_none());
    let mut large = test_pending("large");
    large.request.tool_input["description"] = json!("x".repeat(MAX_RECORD));
    service
        .update_pending(&mut out, &mut watch, Some(PendingInteraction::Approval(large)))
        .unwrap();
    assert!(out.is_empty());
}

#[test]
fn approval_input_failures_clear_pending_without_timeline_unavailable() {
    for failure in ["spool", "screen", "dialog"] {
        let (mut service, root, herdr, _, _) = approval_fixture("claude");
        assert_eq!(interactions(&mut service).len(), 1);
        match failure {
            "spool" => fs::remove_dir_all(root.join("spool")).unwrap(),
            "screen" => herdr.lock().unwrap().read_error = true,
            "dialog" => herdr.lock().unwrap().screen.clear(),
            _ => unreachable!(),
        }
        let mut out = Vec::new();
        service.poll(&mut out).unwrap();
        let emitted = frames(out);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0]["event"], "interaction.resolved");
        assert!(interactions(&mut service).is_empty());
        assert!(herdr.lock().unwrap().keys.is_empty());
        cleanup(&service, root);
    }
}

#[test]
fn approval_unwatch_and_initialize_reset_without_resolved_event() {
    for method in ["unwatch", "initialize"] {
        let (mut service, root, _, _, _) = approval_fixture("claude");
        assert_eq!(interactions(&mut service).len(), 1);
        let mut out = Vec::new();
        service
            .request(
                json!({"id":"reset","method":method,"params":{"protocol":1}})
                    .to_string()
                    .as_bytes(),
                &mut out,
            )
            .unwrap();
        let emitted = frames(out);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0]["id"], "reset");
        assert!(service.watch.is_none());
        cleanup(&service, root);
    }
}

#[test]
fn approval_uses_newest_pending_even_when_only_older_dialog_matches() {
    for agent in ["claude", "codex"] {
        let (mut service, root, herdr, mut hook, path) = approval_fixture(agent);
        let first = interactions(&mut service).remove(0);
        let old_command = hook["payload"]["tool_input"]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        hook["payload"]["tool_input"]["command"] = json!("echo newer");
        let call = if agent == "claude" {
            json!({"type":"assistant","uuid":"newer-message","sessionId":hook["payload"]["session_id"],"timestamp":"1970-01-01T00:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"newest","name":"Bash","input":hook["payload"]["tool_input"]}]}})
        } else {
            json!({"type":"response_item","timestamp":"1970-01-01T00:00:00Z","payload":{"type":"custom_tool_call","id":"newest-item","name":"exec","call_id":"newest","input":"exec({cmd:\"echo newer\"})","internal_chat_message_metadata_passthrough":{"turn_id":hook["payload"]["turn_id"]}}})
        };
        let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
        writeln!(file, "{call}").unwrap();
        let record = json!({"v":1,"t":hook["t"].as_f64().unwrap() + 1.0,"agent":agent,"event":"PermissionRequest","session_id":hook["payload"]["session_id"],"payload":hook["payload"]});
        let spool = root.join(format!(
            "spool/{agent}-{}.jsonl",
            hook["payload"]["session_id"].as_str().unwrap()
        ));
        let mut file = fs::OpenOptions::new().append(true).open(spool).unwrap();
        writeln!(file, "{record}").unwrap();
        let emitted = interactions(&mut service);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0]["event"], "interaction.resolved");
        assert_eq!(
            emitted[0]["data"]["request_id"],
            first["data"]["request_id"]
        );
        {
            let mut state = herdr.lock().unwrap();
            state.screen = state.screen.replace(&old_command, "echo newer");
        }
        let emitted = interactions(&mut service);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0]["event"], "interaction.pending");
        assert_eq!(emitted[0]["data"]["request_id"], "newest");
        assert_eq!(emitted[0]["data"]["command"], "echo newer");
        cleanup(&service, root);
    }
}

#[test]
fn approval_newer_command_fixture_is_correlated_and_parses() {
    for agent in ["claude", "codex"] {
        let (hooks, source, screen) = if agent == "claude" {
            (
                include_str!("../../compatibility/phase1/fixtures/claude-hooks.jsonl"),
                include_str!("../../compatibility/phase1/fixtures/claude-transcript.jsonl"),
                include_str!("../../compatibility/phase1/fixtures/claude-approval-screen.txt"),
            )
        } else {
            (
                include_str!("../../compatibility/phase1/fixtures/codex-hooks.jsonl"),
                include_str!("../../compatibility/phase1/fixtures/codex-pending-rollout.jsonl"),
                include_str!("../../compatibility/phase1/fixtures/codex-approval-screen.txt"),
            )
        };
        let mut hook: Value = hooks
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|r| r["event"] == "PermissionRequest" && r["payload"]["tool_name"] == "Bash")
            .unwrap();
        let mut history: Vec<Value> = source
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        if agent == "claude" {
            let end = history
                .iter()
                .position(|r| {
                    r["message"]["content"]
                        .as_array()
                        .is_some_and(|b| b.iter().any(|b| b["type"] == "tool_result"))
                })
                .unwrap();
            history.truncate(end);
        }
        let old_command = hook["payload"]["tool_input"]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        let old = json!({"v":1,"t":hook["t"],"agent":agent,"event":"PermissionRequest","session_id":hook["payload"]["session_id"],"payload":hook["payload"]});
        hook["payload"]["tool_input"]["command"] = json!("echo newer");
        history.push(if agent == "claude" {
            json!({"type":"assistant","uuid":"newer-message","sessionId":hook["payload"]["session_id"],"timestamp":"1970-01-01T00:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"newest","name":"Bash","input":hook["payload"]["tool_input"]}]}})
        } else {
            json!({"type":"response_item","timestamp":"1970-01-01T00:00:00Z","payload":{"type":"custom_tool_call","id":"newest-item","name":"exec","call_id":"newest","input":"exec({cmd:\"echo newer\"})","internal_chat_message_metadata_passthrough":{"turn_id":hook["payload"]["turn_id"]}}})
        });
        let new = json!({"v":1,"t":hook["t"].as_f64().unwrap()+1.0,"agent":agent,"event":"PermissionRequest","session_id":hook["payload"]["session_id"],"payload":hook["payload"]});
        let mut timeline = crate::claude::Transcript::default();
        let session = hook["payload"]["session_id"].as_str().unwrap();
        for (offset, record) in history.iter().enumerate() {
            let result = if agent == "claude" {
                timeline.apply(record, session)
            } else {
                timeline.apply_codex(record, session, offset)
            };
            assert!(result.is_ok(), "{agent}: {result:?}, record: {record}");
        }
        let matches = if agent == "claude" {
            approval_match::match_claude_records(&[old, new], &history)
        } else {
            approval_match::match_codex_records(&[old, new], &history)
        };
        assert_eq!(matches.last().unwrap().request_id, "newest", "{agent}");
        let updated = screen.replace(&old_command, "echo newer");
        assert_ne!(updated, screen, "{agent}");
        assert!(
            approval_screen::parse(Agent::parse(agent).unwrap(), &updated, "echo newer").is_some(),
            "{agent}: {updated}"
        );
    }
}

#[test]
fn approval_source_changing_preserves_pending_without_flapping() {
    for agent in ["claude", "codex"] {
        let (mut service, root, _, _, _) = approval_fixture(agent);
        let first = interactions(&mut service).remove(0);
        for _ in 0..11 {
            let mut out = Vec::new();
            service
                .poll_with_refresh(&mut out, |_, _, _| Err("source_changing"))
                .unwrap();
            assert!(
                frames(out)
                    .iter()
                    .all(|f| !f["event"].as_str().unwrap().starts_with("interaction."))
            );
            assert_eq!(
                service
                    .watch
                    .as_ref()
                    .unwrap()
                    .pending
                    .as_ref()
                    .unwrap()
                    .data(),
                first["data"]
            );
        }
        assert!(interactions(&mut service).is_empty());
        cleanup(&service, root);
    }
}

#[test]
fn approval_unchanged_blocked_poll_reuses_match_but_rereads_screen() {
    for agent in ["claude", "codex"] {
        let (mut service, root, herdr, _, _) = approval_fixture(agent);
        assert_eq!(interactions(&mut service).len(), 1);
        let stamp = service
            .watch
            .as_ref()
            .unwrap()
            .approval_cache
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
                .approval_cache
                .as_ref()
                .unwrap()
                .stamp,
            stamp
        );
        herdr.lock().unwrap().screen.clear();
        assert_eq!(
            interactions(&mut service)[0]["event"],
            "interaction.resolved"
        );
        assert_eq!(herdr.lock().unwrap().reads, reads + 2);
        assert!(service.watch.as_ref().unwrap().approval_cache.is_some());
        herdr.lock().unwrap().status = Some("idle".into());
        assert!(interactions(&mut service).is_empty());
        assert!(service.watch.as_ref().unwrap().approval_cache.is_none());
        cleanup(&service, root);
    }
}

#[test]
fn approval_match_cache_skips_tail_and_invalidates_on_either_input_change() {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "approval-cache-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    let path = root.join("transcript.jsonl");
    let pending = test_pending("one");
    let identity = pending.identity;
    let input = pending.request.tool_input;
    write_records(
        &path,
        &[
            json!({"type":"assistant","timestamp":"1970-01-01T00:00:00Z","sessionId":SESSION,"uuid":"message","message":{"role":"assistant","content":[{"type":"tool_use","id":"one","name":"Bash","input":input}]}}),
        ],
    );
    let spool_path = root.join(format!("claude-{SESSION}.jsonl"));
    let mut hook = json!({"v":1,"t":1.0,"agent":"claude","event":"PermissionRequest","session_id":SESSION,"payload":{"session_id":SESSION,"tool_name":"Bash","tool_input":input}});
    write_records(&spool_path, &[hook.clone()]);
    let mut service = Service::with_resolvers(root.join("unused.sock"), None, None);
    service.spool_dir = Some(root.clone());
    let mut cache = None;
    let reads = std::cell::Cell::new(0);
    let tail = |file| {
        reads.set(reads.get() + 1);
        crate::follower::read_tail(file)
    };
    assert_eq!(
        service
            .matched_request(&identity, fs::File::open(&path).unwrap(), &mut cache, tail)
            .unwrap()
            .request_id,
        "one"
    );
    assert_eq!(reads.get(), 1);
    assert_eq!(
        service
            .matched_request(&identity, fs::File::open(&path).unwrap(), &mut cache, tail)
            .unwrap()
            .request_id,
        "one"
    );
    assert_eq!(reads.get(), 1);
    hook["t"] = json!(2.0);
    hook["payload"]["permission_suggestions"] = json!(["updated"]);
    write_records(&spool_path, &[hook]);
    assert_eq!(
        service
            .matched_request(&identity, fs::File::open(&path).unwrap(), &mut cache, tail)
            .unwrap()
            .suggestions,
        Some(json!(["updated"]))
    );
    assert_eq!(reads.get(), 2);
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"one"}]}})
    )
    .unwrap();
    assert!(
        service
            .matched_request(&identity, fs::File::open(&path).unwrap(), &mut cache, tail)
            .is_none()
    );
    assert_eq!(reads.get(), 3);
    assert!(
        service
            .matched_request(&identity, fs::File::open(&path).unwrap(), &mut cache, tail)
            .is_none()
    );
    assert_eq!(reads.get(), 3);
    let mut other_identity = identity.clone();
    other_identity.terminal_id = "replacement-terminal".into();
    assert!(
        service
            .matched_request(
                &other_identity,
                fs::File::open(&path).unwrap(),
                &mut cache,
                tail
            )
            .is_none()
    );
    assert_eq!(reads.get(), 4);
    assert!(cache.as_ref().unwrap().identity == other_identity);
    fs::remove_file(spool_path).unwrap();
    assert!(
        service
            .matched_request(&identity, fs::File::open(&path).unwrap(), &mut cache, tail)
            .is_none()
    );
    assert!(cache.is_none());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn approvals_require_client_opt_in() {
    let (mut service, root, herdr, _, _) = approval_fixture("claude");
    let mut out = Vec::new();
    service.request(br#"{"id":"i","method":"initialize","params":{"protocol":1}}"#, &mut out).unwrap();
    assert_eq!(frames(out)[0]["result"]["capabilities"]["approvals"], true);
    service.request(br#"{"id":"w","method":"watch","params":{"pane_id":"w1:p1"}}"#, &mut Vec::new()).unwrap();
    let reads = herdr.lock().unwrap().reads;
    assert!(interactions(&mut service).is_empty());
    assert_eq!(herdr.lock().unwrap().reads, reads);
    let mut out = Vec::new();
    service.request(br#"{"id":"r","method":"interaction.respond","params":{}}"#, &mut out).unwrap();
    assert_eq!(frames(out)[0]["error"]["code"], "unsupported_capability");
    let mut out = Vec::new();
    service.request(br#"{"id":"i","method":"initialize","params":{"protocol":1,"approvals":true}}"#, &mut out).unwrap();
    service.request(br#"{"id":"w","method":"watch","params":{"pane_id":"w1:p1"}}"#, &mut out).unwrap();
    assert!(frames(out).iter().any(|f| f["event"] == "interaction.pending"));
    cleanup(&service, root);
}
