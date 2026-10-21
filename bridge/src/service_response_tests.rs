// Included inside service::tests to use the fake Herdr and approval fixtures.
fn response_params(pending: &Value, number: u32) -> Value {
    let option = pending["data"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .find(|option| option["number"] == number)
        .unwrap();
    json!({"request_id":pending["data"]["request_id"],"option":number,"label_lines":option["label_lines"]})
}
fn respond(service: &mut Service, params: Value) -> Vec<Value> {
    let mut out = Vec::new();
    service
        .request(
            json!({"id":"response","method":"interaction.respond","params":params})
                .to_string()
                .as_bytes(),
            &mut out,
        )
        .unwrap();
    frames(out)
}

#[test]
fn response_requires_watch_identity_and_emitted_request() {
    let mut service = Service::with_resolvers(PathBuf::from("unused.sock"), None, None);
    service.initialized = true;
    assert_eq!(respond(&mut service, json!({}))[0]["error"]["code"], "unsupported_capability");
    service.include_approvals = true;
    assert_eq!(
        respond(&mut service, json!({}))[0]["error"]["code"],
        "not_watching"
    );
    service.watch = Some(test_watch());
    assert_eq!(
        respond(&mut service, json!({}))[0]["error"]["code"],
        "timeline_unavailable"
    );
    service.watch.as_mut().unwrap().identity = Some(test_pending("one").identity);
    assert_eq!(
        respond(&mut service, json!({"request_id":"one"}))[0]["error"]["code"],
        "stale"
    );
    service.watch.as_mut().unwrap().pending = Some(test_pending("one"));
    assert_eq!(
        respond(&mut service, json!({"request_id":"different"}))[0]["error"]["code"],
        "stale"
    );
}

#[test]
fn response_shortcuts_validate_complete_label_rows_and_option_numbers() {
    let pending = test_pending("one");
    let option = &pending.dialog.options[1];
    assert_eq!(
        pending.response_key(&json!({"option":2,"label_lines":option.label_lines})),
        Ok("2".to_string())
    );
    for params in [
        json!({"option":0,"label_lines":option.label_lines}),
        json!({"option":99,"label_lines":option.label_lines}),
        json!({"option":-1,"label_lines":option.label_lines}),
        json!({"option":"2","label_lines":option.label_lines}),
        json!({"option":2}),
        json!({"option":2,"label_lines":option.label_lines.join(" ")}),
        json!({"option":2,"label_lines":[option.label_lines.join(" ")]}),
        json!({"option":2,"label_lines":["wrong"]}),
    ] {
        assert_eq!(pending.response_key(&params), Err("mismatch"), "{params}");
    }
}

#[test]
fn response_sends_exactly_one_shortcut_and_then_resolves() {
    for (agent, number, key) in [
        ("claude", 1, "1"),
        ("claude", 2, "2"),
        ("codex", 1, "y"),
        ("codex", 2, "p"),
        ("codex", 3, "esc"),
    ] {
        let (mut service, root, herdr, _, _) = approval_fixture(agent);
        let pending = interactions(&mut service).remove(0);
        herdr
            .lock()
            .unwrap()
            .screens_after_send
            .push_back(String::new());
        let emitted = respond(&mut service, response_params(&pending, number));
        assert_eq!(
            emitted[0],
            json!({"id":"response","result":{"request_id":pending["data"]["request_id"],"sent":key,"confirmed":true}})
        );
        assert_eq!(emitted[1]["event"], "interaction.resolved");
        assert_eq!(
            emitted[1]["data"]["request_id"],
            pending["data"]["request_id"]
        );
        assert_eq!(
            herdr.lock().unwrap().keys,
            vec![json!({"pane_id":"w1:p1","keys":[key]})]
        );
        cleanup(&service, root);
    }
}

#[test]
fn response_accepts_highlight_change_and_waits_for_dialog_to_disappear() {
    for agent in ["claude", "codex"] {
        let (mut service, root, herdr, _, _) = approval_fixture(agent);
        let pending = interactions(&mut service).remove(0);
        {
            let mut state = herdr.lock().unwrap();
            state.screen = if agent == "claude" {
                state.screen.replace("❯ 1.", "  1.").replace("  2.", "❯ 2.")
            } else {
                state.screen.replace("› 1.", "  1.").replace("  2.", "› 2.")
            };
            let selected = state.screen.clone();
            state.screens_after_send.extend([selected, String::new()]);
        }
        let emitted = respond(&mut service, response_params(&pending, 1));
        assert_eq!(emitted[0]["result"]["confirmed"], true);
        assert_eq!(herdr.lock().unwrap().keys.len(), 1);
        cleanup(&service, root);
    }
}

#[test]
fn response_unchanged_dialog_returns_unconfirmed_without_resending() {
    let (mut service, root, herdr, _, _) = approval_fixture("claude");
    let pending = interactions(&mut service).remove(0);
    let emitted = respond(&mut service, response_params(&pending, 1));
    assert_eq!(
        emitted,
        vec![
            json!({"id":"response","result":{"request_id":pending["data"]["request_id"],"sent":"1","confirmed":false}})
        ]
    );
    assert_eq!(
        herdr.lock().unwrap().keys,
        vec![json!({"pane_id":"w1:p1","keys":["1"]})]
    );
    cleanup(&service, root);
}

#[test]
fn response_refuses_stale_screen_or_mismatched_option_without_keys() {
    for (case, code) in [
        ("request", "stale"),
        ("screen", "stale"),
        ("ambiguous", "stale"),
        ("changed_label", "stale"),
        ("read_error", "stale"),
        ("option", "mismatch"),
        ("labels", "mismatch"),
    ] {
        let (mut service, root, herdr, _, _) = approval_fixture("claude");
        let pending = interactions(&mut service).remove(0);
        let mut params = response_params(&pending, 1);
        {
            let mut state = herdr.lock().unwrap();
            match case {
                "request" => {
                    params["request_id"] = json!("old");
                    state.read_error = true;
                }
                "screen" => state.screen = state.screen.replace("herdcats-probe", "other"),
                "ambiguous" => state.screen = format!("{}\n{}", state.screen, state.screen),
                "changed_label" => state.screen = state.screen.replace("1. Yes", "1. Allow once"),
                "read_error" => state.read_error = true,
                "option" => params["option"] = json!(99),
                "labels" => params["label_lines"] = json!(["different"]),
                _ => unreachable!(),
            }
        }
        let reads = herdr.lock().unwrap().reads;
        let emitted = respond(&mut service, params);
        assert_eq!(
            emitted,
            vec![json!({"id":"response","error":{"code":code}})],
            "{case}"
        );
        assert!(herdr.lock().unwrap().keys.is_empty(), "{case}");
        if case == "request" {
            assert_eq!(herdr.lock().unwrap().reads, reads);
        }
        cleanup(&service, root);
    }
}

#[test]
fn response_refuses_resolved_aborted_or_missing_inputs_without_keys() {
    for (agent, case) in [
        ("claude", "output"),
        ("codex", "output"),
        ("codex", "abort"),
        ("claude", "spool"),
        ("codex", "transcript"),
    ] {
        let (mut service, root, herdr, hook, path) = approval_fixture(agent);
        let pending = interactions(&mut service).remove(0);
        if case == "spool" {
            fs::remove_dir_all(root.join("spool")).unwrap();
        } else if case == "transcript" {
            fs::remove_file(&path).unwrap();
        } else {
            let id = &pending["data"]["request_id"];
            let record = if agent == "claude" {
                json!({"type":"user","uuid":"result","sessionId":hook["payload"]["session_id"],"message":{"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":"done"}]}})
            } else if case == "abort" {
                json!({"type":"event_msg","payload":{"type":"turn_aborted","turn_id":hook["payload"]["turn_id"]}})
            } else {
                json!({"type":"response_item","payload":{"type":"custom_tool_call_output","call_id":id,"output":"done"}})
            };
            let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(file, "{record}").unwrap();
        }
        assert_eq!(
            respond(&mut service, response_params(&pending, 1)),
            vec![json!({"id":"response","error":{"code":"stale"}})],
            "{agent}/{case}"
        );
        assert!(herdr.lock().unwrap().keys.is_empty());
        cleanup(&service, root);
    }
}

#[test]
fn response_refuses_newest_call_with_same_command_and_bypasses_poll_cache() {
    for agent in ["claude", "codex"] {
        let (mut service, root, herdr, hook, path) = approval_fixture(agent);
        let pending = interactions(&mut service).remove(0);
        let call = if agent == "claude" {
            json!({"type":"assistant","uuid":"newest-message","sessionId":hook["payload"]["session_id"],"timestamp":"1970-01-01T00:00:00Z","message":{"role":"assistant","content":[{"type":"tool_use","id":"newest","name":"Bash","input":hook["payload"]["tool_input"]}]}})
        } else {
            let command = serde_json::to_string(&hook["payload"]["tool_input"]["command"]).unwrap();
            json!({"type":"response_item","timestamp":"1970-01-01T00:00:00Z","payload":{"type":"custom_tool_call","id":"newest-item","name":"exec","call_id":"newest","input":format!("exec({{cmd:{command}}})"),"internal_chat_message_metadata_passthrough":{"turn_id":hook["payload"]["turn_id"]}}})
        };
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{call}").unwrap();
        let record = json!({"v":1,"t":hook["t"].as_f64().unwrap()+1.0,"agent":agent,"event":"PermissionRequest","session_id":hook["payload"]["session_id"],"payload":hook["payload"]});
        let mut spool = fs::OpenOptions::new()
            .append(true)
            .open(root.join(format!(
                "spool/{agent}-{}.jsonl",
                hook["payload"]["session_id"].as_str().unwrap()
            )))
            .unwrap();
        writeln!(spool, "{record}").unwrap();
        // Make even a stamp-valid polling cache contain the previously emitted
        // call. A response must re-read and re-match instead of trusting it.
        let identity = service.watch.as_ref().unwrap().identity.clone().unwrap();
        let stamp = service
            .approval_input_stamp(&identity, &fs::File::open(&path).unwrap())
            .unwrap();
        service
            .watch
            .as_mut()
            .unwrap()
            .approval_cache
            .as_mut()
            .unwrap()
            .stamp = stamp;
        assert_eq!(
            respond(&mut service, response_params(&pending, 1)),
            vec![json!({"id":"response","error":{"code":"stale"}})]
        );
        assert!(herdr.lock().unwrap().keys.is_empty());
        cleanup(&service, root);
    }
}

#[test]
fn response_refuses_unblocked_missing_or_replaced_pane_without_keys() {
    for case in ["idle", "missing", "terminal", "agent", "session"] {
        let (mut service, root, herdr, _, _) = approval_fixture("claude");
        let pending = interactions(&mut service).remove(0);
        {
            let mut state = herdr.lock().unwrap();
            match case {
                "idle" => state.status = Some("idle".into()),
                "missing" => state.hide_pane = true,
                "terminal" => state.terminal_id = Some("replacement".into()),
                "agent" => state.agent = Some("codex".into()),
                "session" => state.session_override = Some(OTHER.into()),
                _ => unreachable!(),
            }
        }
        assert_eq!(
            respond(&mut service, response_params(&pending, 1)),
            vec![json!({"id":"response","error":{"code":"stale"}})],
            "{case}"
        );
        assert!(herdr.lock().unwrap().keys.is_empty());
        cleanup(&service, root);
    }
}

#[test]
fn response_shortcuts_from_both_fixture_dialogs_never_include_enter() {
    for (agent, screen, command, expected) in [
        (
            Agent::Claude,
            include_str!("../../compatibility/phase1/fixtures/claude-approval-screen.txt"),
            "echo herdcats-probe > probe.txt",
            vec!["1", "2", "3", "4"],
        ),
        (
            Agent::Codex,
            include_str!("../../compatibility/phase1/fixtures/codex-approval-screen.txt"),
            "echo herdcats-probe > probe2.txt",
            vec!["y", "p", "esc"],
        ),
    ] {
        let mut pending = test_pending("one");
        pending.request.agent = agent;
        pending.dialog = approval_screen::parse(agent, screen, command).unwrap();
        for (option, expected) in pending.dialog.options.iter().zip(expected) {
            let key = pending
                .response_key(&json!({"option":option.number,"label_lines":option.label_lines}))
                .unwrap();
            assert_eq!(key, expected);
            assert!(!["enter", "\n", "\r"].contains(&key.as_str()));
        }
    }
}

#[test]
fn response_failed_confirmation_read_returns_unconfirmed_after_one_key() {
    let (mut service, root, herdr, _, _) = approval_fixture("codex");
    let pending = interactions(&mut service).remove(0);
    herdr.lock().unwrap().read_error_after_send = true;
    let emitted = respond(&mut service, response_params(&pending, 1));
    assert_eq!(
        emitted[0],
        json!({"id":"response","result":{"request_id":pending["data"]["request_id"],"sent":"y","confirmed":false}})
    );
    assert_eq!(
        herdr.lock().unwrap().keys,
        vec![json!({"pane_id":"w1:p1","keys":["y"]})]
    );
    cleanup(&service, root);
}
