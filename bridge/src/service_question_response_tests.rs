// Real response requests against the shared fake Herdr, with renders scripted per input.
fn question_screen_fixture(name: &str) -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../compatibility/phase2/fixtures/{name}-screen.txt"
    )))
    .unwrap()
}
fn script_question(herdr: &SharedHerdr, screens: Vec<String>) {
    let mut state = herdr.lock().unwrap();
    state.allow_question_input = true;
    state.screens_after_input = screens.into_iter().map(|s| [s].into()).collect();
}
fn question_params(pending: &Value, answers: Value) -> Value {
    json!({"request_id":pending["data"]["request_id"],"answers":answers})
}
fn keys_input(keys: &[&str]) -> Value {
    json!({"method":"pane.send_keys","params":{"pane_id":"w1:p1","keys":keys}})
}
fn text_input(text: &str) -> Value {
    json!({"method":"pane.send_text","params":{"pane_id":"w1:p1","text":text}})
}
fn assert_question_success(frames: &[Value], pending: &Value) {
    assert_eq!(
        frames[0],
        json!({"id":"response","result":{"request_id":pending["data"]["request_id"],"confirmed":true}})
    );
    assert!(frames.iter().any(|f| f["event"] == "interaction.resolved"
        && f["data"]["request_id"] == pending["data"]["request_id"]));
}
#[test]
fn question_response_claude_runs_tabs_text_review_and_submit() {
    let (mut service, root, herdr, _) = question_fixture("claude");
    let pending = interactions(&mut service).remove(0);
    let multi = question_screen_fixture("claude-question-multi");
    let pristine = multi
        .replace("☒ Toppings", "☐ Toppings")
        .replace("[✔]", "[ ]");
    let cheese = multi.replace("3. [✔] Basil", "3. [ ] Basil");
    let other = multi.replace("❯ 1.", "  1.").replace("  4.", "❯ 4.");
    let typed = question_screen_fixture("claude-question-other");
    let submit = typed
        .replace("❯ 4.", "  4.")
        .replace("     Submit", "❯    Submit");
    script_question(
        &herdr,
        vec![
            pristine,
            cheese,
            multi,
            other,
            typed,
            submit,
            question_screen_fixture("claude-question-review"),
            String::new(),
        ],
    );
    let emitted = respond(
        &mut service,
        question_params(
            &pending,
            json!([{"index":0,"options":[2]},{"index":1,"options":[1,3],"text":"Pepper"}]),
        ),
    );
    assert_question_success(&emitted, &pending);
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            keys_input(&["2"]),
            keys_input(&["1"]),
            keys_input(&["3"]),
            keys_input(&["down", "down", "down"]),
            text_input("Pepper"),
            keys_input(&["down"]),
            keys_input(&["enter"]),
            keys_input(&["1"])
        ]
    );
    cleanup(&service, root);
}
#[test]
fn question_response_codex_plan_runs_notes_and_waits_for_render() {
    let (mut service, root, herdr, _) = question_fixture("codex_plan");
    let pending = interactions(&mut service).remove(0);
    let notes = question_screen_fixture("codex-sync-notes");
    script_question(
        &herdr,
        vec![
            question_screen_fixture("codex-sync-last"),
            notes.replace("› Cheese, Basil", "›"),
            notes,
            String::new(),
        ],
    );
    // A terminal may retain the previous render until the next 100ms poll.
    let first = herdr.lock().unwrap().screen.clone();
    herdr.lock().unwrap().screens_after_input[0].push_front(first);
    let emitted = respond(
        &mut service,
        question_params(
            &pending,
            json!([{"index":0,"options":[1]},{"index":1,"options":[1],"text":"Cheese, Basil"}]),
        ),
    );
    assert_question_success(&emitted, &pending);
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            keys_input(&["1"]),
            keys_input(&["tab"]),
            text_input("Cheese, Basil"),
            keys_input(&["enter"])
        ]
    );
    cleanup(&service, root);
}
#[test]
fn question_response_async_opens_other_types_and_submits() {
    let (mut service, root, herdr, _) = question_fixture("codex_async");
    let pending = interactions(&mut service).remove(0);
    let picker = question_screen_fixture("codex-async-picker");
    let other = picker.replace("› 1.", "  1.").replace("  3.", "› 3.");
    script_question(
        &herdr,
        vec![
            picker,
            other,
            question_screen_fixture("codex-async-other"),
            String::new(),
        ],
    );
    let emitted = respond(
        &mut service,
        question_params(&pending, json!([{"index":0,"options":[],"text":"Green"}])),
    );
    assert_question_success(&emitted, &pending);
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            keys_input(&["shift+left"]),
            keys_input(&["3"]),
            text_input("Green"),
            keys_input(&["enter"])
        ]
    );
    cleanup(&service, root);
}
#[test]
fn question_response_stops_on_mismatch_with_steps_sent() {
    let (mut service, root, herdr, _) = question_fixture("claude");
    let pending = interactions(&mut service).remove(0);
    let multi = question_screen_fixture("claude-question-multi");
    script_question(
        &herdr,
        vec![
            multi
                .replace("☒ Toppings", "☐ Toppings")
                .replace("[✔]", "[ ]"),
            multi,
        ],
    );
    let emitted = respond(
        &mut service,
        question_params(
            &pending,
            json!([{"index":0,"options":[2]},{"index":1,"options":[1,3],"text":"Pepper"}]),
        ),
    );
    assert_eq!(
        emitted,
        vec![json!({"id":"response","error":{"code":"stale","steps_sent":2}})]
    );
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![keys_input(&["2"]), keys_input(&["1"])]
    );
    cleanup(&service, root);
}
#[test]
fn question_response_requires_both_emitted_and_live_freshness() {
    for emit_changed in [false, true] {
        let (mut service, root, herdr, _) = question_fixture("claude");
        let pending = interactions(&mut service).remove(0);
        herdr.lock().unwrap().screen = question_screen_fixture("claude-question-multi");
        if emit_changed {
            assert_eq!(interactions(&mut service)[0]["data"]["fresh"], false);
        }
        if emit_changed {
            herdr.lock().unwrap().screen = question_screen_fixture("claude-question-single");
        }
        let emitted = respond(
            &mut service,
            question_params(
                &pending,
                json!([{"index":0,"options":[2]},{"index":1,"options":[1,3]}]),
            ),
        );
        assert_eq!(
            emitted,
            vec![json!({"id":"response","error":{"code":"in_progress"}})]
        );
        assert!(herdr.lock().unwrap().inputs.is_empty());
        cleanup(&service, root);
    }
}
#[test]
fn question_response_codex_last_digit_only_sends_enter_when_needed() {
    for outcome in ["gone", "selected", "gone_before_enter"] {
        let (mut service, root, herdr, _) = question_fixture("codex_plan");
        let pending = interactions(&mut service).remove(0);
        let last = question_screen_fixture("codex-sync-last");
        script_question(
            &herdr,
            vec![
                last.clone(),
                if outcome == "gone" {
                    String::new()
                } else {
                    last
                },
                String::new(),
            ],
        );
        if outcome == "gone_before_enter" {
            let mut state = herdr.lock().unwrap();
            state.screens_after_input[1].push_back(String::new());
            state.status_after_inputs = Some((2, "idle".into()));
        }
        let emitted = respond(
            &mut service,
            question_params(
                &pending,
                json!([{"index":0,"options":[1]},{"index":1,"options":[1]}]),
            ),
        );
        assert_question_success(&emitted, &pending);
        let mut want = vec![keys_input(&["1"]), keys_input(&["1"])];
        if outcome == "selected" {
            want.push(keys_input(&["enter"]));
        }
        assert_eq!(herdr.lock().unwrap().inputs, want, "{outcome}");
        cleanup(&service, root);
    }
}

#[test]
fn question_response_rechecks_pending_source_and_live_occupant_without_inputs() {
    for case in [
        "resolved",
        "superseded",
        "changed",
        "missing_source",
        "idle",
        "missing_pane",
        "terminal",
        "agent",
        "session",
        "screen",
        "read_error",
        "request",
        "generation",
    ] {
        let (mut service, root, herdr, path) = question_fixture("claude");
        let pending = interactions(&mut service).remove(0);
        let mut params = question_params(
            &pending,
            json!([{"index":0,"options":[2]},{"index":1,"options":[1]}]),
        );
        match case {
            "resolved" => {
                fs::write(
                    &path,
                    include_str!(
                        "../../compatibility/phase2/fixtures/claude-question-transcript.jsonl"
                    ),
                )
                .unwrap();
            }
            "missing_source" => fs::remove_file(&path).unwrap(),
            "superseded" | "changed" => {
                let mut records: Vec<Value> = fs::read_to_string(&path)
                    .unwrap()
                    .lines()
                    .map(|l| serde_json::from_str(l).unwrap())
                    .collect();
                let mut call = records
                    .iter()
                    .find(|r| {
                        r["message"]["content"].as_array().is_some_and(|blocks| {
                            blocks.iter().any(|b| b["name"] == "AskUserQuestion")
                        })
                    })
                    .unwrap()
                    .clone();
                let block = call["message"]["content"]
                    .as_array_mut()
                    .unwrap()
                    .iter_mut()
                    .find(|b| b["name"] == "AskUserQuestion")
                    .unwrap();
                if case == "superseded" {
                    block["id"] = json!("newest");
                } else {
                    block["input"]["questions"][0]["options"][0]["description"] = json!("changed");
                }
                records.push(call);
                write_records(&path, &records);
                // Even a stamp-valid cache must not authorize the old call.
                let stamp = ApprovalFileStamp::of(&fs::metadata(&path).unwrap());
                service
                    .watch
                    .as_mut()
                    .unwrap()
                    .question_cache
                    .as_mut()
                    .unwrap()
                    .stamp = stamp;
            }
            "idle" => herdr.lock().unwrap().status = Some("idle".into()),
            "missing_pane" => herdr.lock().unwrap().hide_pane = true,
            "terminal" => herdr.lock().unwrap().terminal_id = Some("replacement".into()),
            "agent" => herdr.lock().unwrap().agent = Some("codex".into()),
            "session" => herdr.lock().unwrap().session_override = Some(OTHER.into()),
            "screen" => herdr.lock().unwrap().screen.clear(),
            "read_error" => herdr.lock().unwrap().read_error = true,
            "request" => params["request_id"] = json!("old"),
            "generation" => service.generation += 1,
            _ => unreachable!(),
        }
        assert_eq!(
            respond(&mut service, params),
            vec![json!({"id":"response","error":{"code":"stale"}})],
            "{case}"
        );
        assert!(herdr.lock().unwrap().inputs.is_empty(), "{case}");
        cleanup(&service, root);
    }
}
#[test]
fn question_response_errors_preserve_independent_capabilities_and_answer_validation() {
    let mut service = Service::with_resolvers(PathBuf::from("unused.sock"), None, None);
    service.initialized = true;
    service.include_questions = true;
    assert_eq!(
        respond(&mut service, json!({"answers":[]}))[0]["error"]["code"],
        "not_watching"
    );
    service.watch = Some(test_watch());
    assert_eq!(
        respond(&mut service, json!({"answers":[]}))[0]["error"]["code"],
        "timeline_unavailable"
    );
    for (case, code) in [
        ("opt_out", "unsupported_capability"),
        ("missing", "invalid"),
        ("invalid", "invalid"),
        ("mismatch", "mismatch"),
    ] {
        let (mut service, root, herdr, _) = question_fixture("claude");
        let pending = interactions(&mut service).remove(0);
        let mut params = question_params(
            &pending,
            json!([{"index":0,"options":[2]},{"index":1,"options":[1]}]),
        );
        match case {
            "opt_out" => service.include_questions = false,
            "missing" => {
                params.as_object_mut().unwrap().remove("answers");
            }
            "invalid" => params["answers"][0]["text"] = json!("line\nbreak"),
            "mismatch" => params["answers"][0]["text"] = json!("extra"),
            _ => unreachable!(),
        }
        assert_eq!(
            respond(&mut service, params),
            vec![json!({"id":"response","error":{"code":code}})],
            "{case}"
        );
        assert!(herdr.lock().unwrap().inputs.is_empty());
        cleanup(&service, root);
    }
}
#[test]
fn question_response_guard_read_catches_desk_edit_before_next_input() {
    let (mut service, root, herdr, _) = question_fixture("codex_async");
    let pending = interactions(&mut service).remove(0);
    let picker = question_screen_fixture("codex-async-picker");
    script_question(&herdr, vec![picker.clone()]);
    herdr.lock().unwrap().screens_after_input[0]
        .push_back(picker.replace("› 1.", "  1.").replace("  2.", "› 2."));
    assert_eq!(
        respond(
            &mut service,
            question_params(&pending, json!([{"index":0,"options":[1]}]))
        ),
        vec![json!({"id":"response","error":{"code":"stale","steps_sent":1}})]
    );
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![keys_input(&["shift+left"])]
    );
    cleanup(&service, root);
}
#[test]
fn question_response_stops_if_occupant_changes_between_steps() {
    let (mut service, root, herdr, _) = question_fixture("codex_async");
    let pending = interactions(&mut service).remove(0);
    script_question(&herdr, vec![question_screen_fixture("codex-async-picker")]);
    herdr.lock().unwrap().terminal_after_input = Some("replacement".into());
    assert_eq!(
        respond(
            &mut service,
            question_params(&pending, json!([{"index":0,"options":[1]}]))
        ),
        vec![json!({"id":"response","error":{"code":"stale","steps_sent":1}})]
    );
    assert_eq!(herdr.lock().unwrap().inputs.len(), 1);
    cleanup(&service, root);
}
#[test]
fn question_response_read_failure_midway_is_stale_but_final_confirmation_can_fail() {
    for final_input in [false, true] {
        let (mut service, root, herdr, _) = question_fixture("codex_async");
        if final_input {
            herdr.lock().unwrap().screen = question_screen_fixture("codex-async-picker");
        }
        let pending = interactions(&mut service).remove(0);
        script_question(&herdr, vec![]);
        herdr.lock().unwrap().read_error_after_send = true;
        let params = if final_input {
            json!({"request_id":pending["data"]["request_id"],"skip":true})
        } else {
            question_params(&pending, json!([{"index":0,"options":[1]}]))
        };
        let emitted = respond(&mut service, params);
        if final_input {
            assert_eq!(
                emitted[0],
                json!({"id":"response","result":{"request_id":pending["data"]["request_id"],"confirmed":false}})
            );
        } else {
            assert_eq!(
                emitted,
                vec![json!({"id":"response","error":{"code":"stale","steps_sent":1}})]
            );
        }
        assert_eq!(herdr.lock().unwrap().inputs.len(), 1);
        cleanup(&service, root);
    }
}
#[test]
fn question_response_final_unchanged_dialog_is_unconfirmed_without_resending() {
    let (mut service, root, herdr, _) = question_fixture("codex_async");
    herdr.lock().unwrap().screen = question_screen_fixture("codex-async-picker");
    let pending = interactions(&mut service).remove(0);
    script_question(&herdr, vec![]);
    let started = Instant::now();
    let emitted = respond(
        &mut service,
        json!({"request_id":pending["data"]["request_id"],"skip":true}),
    );
    assert_eq!(
        emitted[0],
        json!({"id":"response","result":{"request_id":pending["data"]["request_id"],"confirmed":false}})
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(herdr.lock().unwrap().inputs, vec![keys_input(&["ctrl+]"])]);
    cleanup(&service, root);
}
#[test]
fn question_response_async_skip_from_banner_opens_then_skips() {
    let (mut service, root, herdr, _) = question_fixture("codex_async");
    let pending = interactions(&mut service).remove(0);
    script_question(
        &herdr,
        vec![question_screen_fixture("codex-async-picker"), String::new()],
    );
    let emitted = respond(
        &mut service,
        json!({"request_id":pending["data"]["request_id"],"skip":true}),
    );
    assert_question_success(&emitted, &pending);
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![keys_input(&["shift+left"]), keys_input(&["ctrl+]"])]
    );
    cleanup(&service, root);
}
