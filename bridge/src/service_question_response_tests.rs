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
    {
        let mut state = herdr.lock().unwrap();
        state.screen = state.screen.replace("9s", "20s");
    }
    let pending = interactions(&mut service).remove(0);
    assert_eq!(pending["data"]["countdown"], "20s");
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
                let identity = service.watch.as_ref().unwrap().identity.clone().unwrap();
                let spool_path =
                    root.join(format!("spool/claude-{}.jsonl", identity.native_session_id));
                let mut records: Vec<Value> = fs::read_to_string(&spool_path)
                    .unwrap()
                    .lines()
                    .map(|l| serde_json::from_str(l).unwrap())
                    .collect();
                let mut record = records.last().unwrap().clone();
                if case == "superseded" {
                    record["t"] = json!(record["t"].as_f64().unwrap() + 1.0);
                } else {
                    record["payload"]["tool_input"]["questions"][0]["options"][0]["description"] =
                        json!("changed");
                }
                records.push(record);
                write_records(&spool_path, &records);
                // A response must ignore even a stamp-valid polling cache.
                let stamp = service
                    .question_input_stamp(&identity, &fs::File::open(&path).unwrap())
                    .unwrap();
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
    let (mut service, root, herdr, path) = question_fixture("codex_async");
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
    let identity = service.watch.as_ref().unwrap().identity.as_ref().unwrap();
    assert!(
        service
            .matched_question(identity, fs::File::open(path).unwrap(), &mut None)
            .is_some()
    );
    assert!(interactions(&mut service).is_empty());
    cleanup(&service, root);
}

fn codex_first_notes_placeholder() -> String {
    let first = question_screen_fixture("codex-sync-first");
    let placeholder = question_screen_fixture("codex-sync-notes-placeholder");
    let (prefix, _) = first.split_once("  tab ").unwrap();
    let (_, notes) = placeholder.split_once("  › Add notes").unwrap();
    format!("{prefix}  › Add notes{notes}")
}
#[test]
fn question_response_codex_plan_accepts_placeholder_and_advances_after_first_notes() {
    let (mut service, root, herdr, _) = question_fixture("codex_plan");
    let pending = interactions(&mut service).remove(0);
    let placeholder = codex_first_notes_placeholder();
    script_question(
        &herdr,
        vec![
            placeholder.clone(),
            placeholder.replace("› Add notes", "› extra cheese please"),
            question_screen_fixture("codex-sync-last"),
            String::new(),
        ],
    );
    let emitted = respond(
        &mut service,
        question_params(
            &pending,
            json!([
                {"index":0,"options":[1],"text":"extra cheese please"}, {"index":1,"options":[2]}
            ]),
        ),
    );
    assert_question_success(&emitted, &pending);
    // The last digit submitted immediately; no optional Enter was sent.
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            keys_input(&["tab"]),
            text_input("extra cheese please"),
            keys_input(&["enter"]),
            keys_input(&["2"])
        ]
    );
    cleanup(&service, root);
}
#[test]
fn question_response_literal_add_notes_stops_at_the_next_text_check() {
    let (mut service, root, herdr, _) = question_fixture("codex_plan");
    let pending = interactions(&mut service).remove(0);
    let placeholder = codex_first_notes_placeholder();
    script_question(&herdr, vec![placeholder.clone(), placeholder]);
    let emitted = respond(
        &mut service,
        question_params(
            &pending,
            json!([
                {"index":0,"options":[1],"text":"Add notes"}, {"index":1,"options":[2]}
            ]),
        ),
    );
    assert_eq!(
        emitted,
        vec![json!({"id":"response","error":{"code":"stale","steps_sent":2}})]
    );
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![keys_input(&["tab"]), text_input("Add notes")]
    );
    cleanup(&service, root);
}

#[test]
fn question_response_one_claude_multiselect_reaches_review_then_submits() {
    let (mut service, root, herdr, _) = question_fixture("claude");
    interactions(&mut service);
    let session = service
        .watch
        .as_ref()
        .unwrap()
        .identity
        .as_ref()
        .unwrap()
        .native_session_id
        .clone();
    let path = root.join(format!("spool/claude-{session}.jsonl"));
    let mut record: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    record["payload"]["tool_input"]["questions"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    write_records(&path, &[record]);
    let screen = question_screen_fixture("claude-question-one-multi");
    herdr.lock().unwrap().screen = screen.clone();
    let pending = interactions(&mut service).remove(0);
    let cheese = screen
        .replace("1. [ ] Cheese", "1. [✔] Cheese")
        .replace("☐ Toppings", "☒ Toppings");
    let both = cheese.replace("3. [ ] Basil", "3. [✔] Basil");
    let submit = both
        .replace("❯ 1.", "  1.")
        .replace("     Submit", "❯    Submit");
    script_question(
        &herdr,
        vec![
            cheese,
            both,
            submit,
            question_screen_fixture("claude-question-one-multi-review"),
            String::new(),
        ],
    );
    let emitted = respond(
        &mut service,
        question_params(&pending, json!([{"index":0,"options":[1,3]}])),
    );
    assert_question_success(&emitted, &pending);
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            keys_input(&["1"]),
            keys_input(&["3"]),
            keys_input(&["down", "down", "down", "down"]),
            keys_input(&["enter"]),
            keys_input(&["1"])
        ]
    );
    cleanup(&service, root);
}

#[test]
fn question_response_async_rechecks_queue_ambiguity_before_any_input() {
    for (calls, picker) in [(2, false), (1, false), (2, true)] {
        let (mut service, root, herdr, path) = question_fixture("codex_async");
        if picker {
            herdr.lock().unwrap().screen = question_screen_fixture("codex-async-picker");
        }
        let pending = interactions(&mut service).remove(0);
        if calls == 2 {
            add_pending_async_before_original(&path, 1);
            // Keep the old request cached under the current stamp. Response must
            // re-count uncached even though the newest request itself is unchanged.
            let identity = service.watch.as_ref().unwrap().identity.clone().unwrap();
            let stamp = service
                .question_input_stamp(&identity, &fs::File::open(&path).unwrap())
                .unwrap();
            service
                .watch
                .as_mut()
                .unwrap()
                .question_cache
                .as_mut()
                .unwrap()
                .stamp = stamp;
        }
        if !picker {
            let mut state = herdr.lock().unwrap();
            state.screen = state.screen.replace("1 question", "2 questions");
        }
        let emitted = respond(
            &mut service,
            question_params(&pending, json!([{"index":0,"options":[1]}])),
        );
        assert_eq!(
            emitted,
            vec![json!({"id":"response","error":{"code":"stale"}})],
            "calls={calls}, picker={picker}"
        );
        assert!(herdr.lock().unwrap().inputs.is_empty());
        cleanup(&service, root);
    }
}

#[test]
fn question_response_plan_is_unaffected_by_other_pending_async_calls() {
    let (mut service, root, herdr, path) = question_fixture("codex_plan");
    let mut records: Vec<Value> = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let async_call: Value =
        include_str!("../../compatibility/phase2/fixtures/codex-async-rollout-pending.jsonl")
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .find(|r| r["payload"]["name"] == "request_user_input_async")
            .unwrap();
    let index = records
        .iter()
        .position(|r| r["payload"]["name"] == "request_user_input")
        .unwrap();
    for i in 0..2 {
        let mut call = async_call.clone();
        call["payload"]["call_id"] = json!(format!("async-{i}"));
        records.insert(index, call);
    }
    write_records(&path, &records);
    let pending = interactions(&mut service).remove(0);
    assert_eq!(pending["data"]["question_kind"], "codex_plan");
    script_question(
        &herdr,
        vec![question_screen_fixture("codex-sync-last"), String::new()],
    );
    let emitted = respond(
        &mut service,
        question_params(
            &pending,
            json!([{"index":0,"options":[1]},{"index":1,"options":[2]}]),
        ),
    );
    assert_question_success(&emitted, &pending);
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![keys_input(&["1"]), keys_input(&["2"])]
    );
    cleanup(&service, root);
}

#[test]
fn question_response_claude_desk_navigation_is_not_fresh_and_sends_no_input() {
    for multi in [false, true] {
        for emit_changed in [false, true] {
            let (mut service, root, herdr, path) = question_fixture("claude");
            if multi {
                let session = path.file_stem().unwrap().to_str().unwrap();
                let spool_path = root.join(format!("spool/claude-{session}.jsonl"));
                let mut record: Value =
                    serde_json::from_str(&fs::read_to_string(&spool_path).unwrap()).unwrap();
                record["payload"]["tool_input"]["questions"]
                    .as_array_mut()
                    .unwrap()
                    .remove(0);
                write_records(&spool_path, &[record]);
                herdr.lock().unwrap().screen = question_screen_fixture("claude-question-one-multi");
            }
            let pending = interactions(&mut service).remove(0);
            assert_eq!(pending["data"]["fresh"], true);
            {
                let mut state = herdr.lock().unwrap();
                let unselected = state.screen.replace("❯ 1.", "  1.");
                state.screen = if multi {
                    unselected.replace("     Submit", "❯    Submit")
                } else {
                    unselected.replace("  2.", "❯ 2.")
                };
            }
            if emit_changed {
                let events = interactions(&mut service);
                assert_eq!(events.len(), 1);
                assert_eq!(events[0]["event"], "interaction.pending");
                assert_eq!(events[0]["data"]["fresh"], false);
            }
            let answers = if multi {
                json!([{"index":0,"options":[1]}])
            } else {
                json!([{"index":0,"options":[1]},{"index":1,"options":[1]}])
            };
            assert_eq!(
                respond(&mut service, question_params(&pending, answers)),
                vec![json!({"id":"response","error":{"code":"in_progress"}})],
                "multi={multi}, emitted={emit_changed}"
            );
            assert!(herdr.lock().unwrap().inputs.is_empty());
            cleanup(&service, root);
        }
    }
}

#[test]
fn question_response_codex_plan_passes_live_non_last_notes_footer() {
    let (mut service, root, herdr, path) = question_fixture("codex_plan");
    let source = fs::read_to_string(&path).unwrap()
        .replace("Choose a small size.", "Choose Small.")
        .replace("Choose a large size.", "Choose Large.");
    fs::write(&path, source).unwrap();
    let first = question_screen_fixture("codex-sync-first")
        .replace("Choose a small size.", "Choose Small.")
        .replace("Choose a large size.", "Choose Large.");
    herdr.lock().unwrap().screen = first.clone();
    let pending = interactions(&mut service).remove(0);
    let notes = question_screen_fixture("codex-sync-notes-first");
    script_question(&herdr, vec![
        first.replace("› 1.", "  1.").replace("  3.", "› 3."),
        notes.clone(),
        notes.replace("› Add notes", "› medium please"),
        question_screen_fixture("codex-sync-last"),
        String::new(),
    ]);
    let emitted = respond(&mut service, question_params(&pending,
        json!([{"index":0,"options":[3],"text":"medium please"},{"index":1,"options":[1]}])));
    assert_question_success(&emitted, &pending);
    assert_eq!(herdr.lock().unwrap().inputs, vec![
        keys_input(&["down", "down"]), keys_input(&["tab"]),
        text_input("medium please"), keys_input(&["enter"]), keys_input(&["1"]),
    ]);
    cleanup(&service, root);
}
