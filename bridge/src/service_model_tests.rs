// Included in service::tests to exercise model control against fake Herdr.

fn claude_idle_screen() -> String {
    "  Claude Code                         v2.1.309\n\
     Opus 5.5 with medium effort · Claude Pro\n\
     ❯ Try \"fix lint errors\"\n\
     ⏵⏵ auto mode on (shift+tab to cycle)\n"
        .to_owned()
}

fn claude_model_screen(command: &str) -> String {
    format!(
        "  Claude Code                         v2.1.309\n\
         ❯ {command}\n\
           ⎿  Set model to Sonnet 5.5 and saved as your default for new sessions\n"
    )
}

fn claude_effort_screen() -> String {
    "  Claude Code                         v2.1.309\n\
     ❯ /effort high\n\
       ⎿  Set effort level to high (saved as your default for new sessions)\n"
        .to_owned()
}

fn claude_invalid_effort_screen() -> String {
    "  Claude Code                         v2.1.309\n\
     ❯ /effort high\n\
       ⎿  Invalid argument: high. Valid options are: low, medium, high, xhigh, max, auto\n"
        .to_owned()
}

fn codex_model_list_screen() -> String {
    "  To get started, describe a task or try one of these commands:\n\
     /model - choose what model and reasoning effort to use\n\
     Select Model and Effort\n\
     › 1. GPT-6.1-Sol (current)  Latest workhorse model for coding and everyday work.\n\
       2. GPT-6-Astra            Frontier intelligence for the most demanding work.\n\
       3. GPT-6-Sol              Previous generation workhorse model.\n\
       4. GPT-6-Luna             Fast and affordable model for easier tasks.\n\
     enter select · esc back"
        .to_owned()
}

/// Cursor rests on the model's default level, as the real picker does.
fn codex_effort_list_screen() -> String {
    "Select Reasoning Level for GPT-6-Luna\n\
       1. Low       Fast responses with lighter reasoning\n\
     › 2. Medium (default)  Balances speed and reasoning depth for everyday tasks\n\
       3. High    Greater reasoning depth for complex problems\n\
       4. Extra high  Extra high reasoning depth for complex problems\n\
       5. More reasoning…  Max consumes usage limits faster\n\
     enter default · s session · esc back"
        .to_owned()
}

fn codex_advanced_screen() -> String {
    "Advanced Reasoning\n\
     ⚠ Consumes usage limits faster\n\
     › 1. Max  For difficult problems when quality matters more than speed\n\
       2. Ultra  Maximum reasoning with automatic task delegation\n\
     enter default · s session · esc back"
        .to_owned()
}

fn codex_composer_toast(slug: &str, effort: &str) -> String {
    format!(
        "› Ask Codex to do anything\n• Model changed to {slug} {effort} for this session only\nGPT-6-Luna {effort} · Context 100% left"
    )
}

const CODEX_THREAD: &str = "aaaabbbb-cccc-4ddd-8eee-ffff00001111";

/// A Claude pane whose transcript carries model/effort records.
fn model_fixture_claude() -> (Service, PathBuf, SharedHerdr, PathBuf) {
    let (mut service, root, _session, herdr) = fixture_with_herdr();
    service.include_model_control = true;
    let path = root.join(format!("projects/one/{SESSION}.jsonl"));
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!(
        "{}\n{}\n",
        json!({"type":"assistant","uuid":"am1","sessionId":SESSION,
               "message":{"model":"claude-opus-5-5","role":"assistant","content":"first"},
               "effort":"medium"}),
        json!({"type":"api-request","sessionId":SESSION,"params":{
               "model":"claude-opus-5-5","output_config":{"effort":"medium"}}})
    ));
    fs::write(&path, text).unwrap();
    herdr.lock().unwrap().screen = claude_idle_screen();
    (service, root, herdr, path)
}

/// A Codex pane with a rollout carrying turn_context records and a catalog.
fn model_fixture_codex() -> (Service, PathBuf, SharedHerdr, PathBuf) {
    let (mut service, root, session, herdr) = fixture_with_herdr();
    service.include_model_control = true;
    fs::create_dir_all(root.join("codex/2026/10/13")).unwrap();
    fs::create_dir_all(root.join("codex-home")).unwrap();
    service = service
        .with_codex_root(root.join("codex"))
        .with_codex_home(root.join("codex-home"));
    fs::write(
        root.join("codex-home/models_cache.json"),
        json!({"models":[
            {"slug":"gpt-6.1-sol","display_name":"GPT-6.1-Sol","priority":1,"visibility":"list",
             "supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"},{"effort":"high"},{"effort":"xhigh"}]},
            {"slug":"gpt-6-astra","display_name":"GPT-6-Astra","priority":2,"visibility":"list",
             "supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"},{"effort":"high"}]},
            {"slug":"gpt-6-luna","display_name":"GPT-6-Luna","priority":4,"visibility":"list",
             "supported_reasoning_levels":[{"effort":"low"},{"effort":"medium"},{"effort":"high"},{"effort":"xhigh"},{"effort":"max"}]},
            {"slug":"gpt-hidden","display_name":"Hidden","priority":0,"visibility":"hide",
             "supported_reasoning_levels":[]}
        ]})
        .to_string(),
    )
    .unwrap();
    fs::write(
        root.join("codex-home/config.toml"),
        "model = \"gpt-6.1-sol\"\nmodel_reasoning_effort = \"low\"\n",
    )
    .unwrap();
    let path = root.join(format!(
        "codex/2026/10/13/rollout-fixture-{CODEX_THREAD}.jsonl"
    ));
    fs::write(
        &path,
        format!(
            "{}\n{}\n",
            json!({"type":"session_meta","payload":{"id":CODEX_THREAD,"cwd":"/fixture/work"}}),
            json!({"type":"turn_context","payload":{"model":"gpt-6.1-sol","effort":"low","cwd":"/fixture/work"}})
        ),
    )
    .unwrap();
    {
        let mut state = herdr.lock().unwrap();
        state.agent = Some("codex".into());
        *session.lock().unwrap() = CODEX_THREAD.into();
        state.screen = "› Ask Codex to do anything\nGPT-6.1-Sol low · Context 100% left".into();
    }
    (service, root, herdr, path)
}

fn model_request(service: &mut Service, method: &str, params: Value) -> Vec<Value> {
    let mut out = Vec::new();
    service
        .request(
            json!({"id":"m","method":method,"params":params})
                .to_string()
                .as_bytes(),
            &mut out,
        )
        .unwrap();
    frames(out)
}

/// The reply frame for our request id; polls may emit events around it.
fn result_frame(reply: &[Value]) -> &Value {
    reply.iter().find(|frame| frame["id"] == "m").unwrap()
}

fn prompt_input(text: &str) -> Value {
    json!({"method":"agent.prompt","params":{"target":"w1:p1","text":text}})
}

fn model_keys_input(keys: &[&str]) -> Value {
    json!({"method":"pane.send_keys","params":{"pane_id":"w1:p1","keys":keys}})
}

fn script_model(herdr: &SharedHerdr, screens: Vec<String>) {
    let mut state = herdr.lock().unwrap();
    state.allow_question_input = true;
    state.screens_after_input = screens.into_iter().map(|screen| [screen].into()).collect();
}

fn expect_pair(model: &str, effort: Option<&str>) -> Value {
    json!({"model":model,"effort":effort})
}

#[test]
fn initialize_advertises_model_control_and_requires_opt_in() {
    let (mut service, root, _, _) = model_fixture_claude();
    service.initialized = false;
    let reply = model_request(
        &mut service,
        "initialize",
        json!({"protocol":1,"model_control":true}),
    );
    assert_eq!(
        result_frame(&reply)["result"]["capabilities"]["model_control"],
        true
    );
    // Without the opt-in the methods are refused.
    service.initialized = true;
    service.include_model_control = false;
    let reply = model_request(&mut service, "model.status", json!({}));
    assert_eq!(
        result_frame(&reply)["error"]["code"],
        "unsupported_capability"
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"opus","expect":expect_pair("claude-opus-5-5", Some("medium"))}),
    );
    assert_eq!(
        result_frame(&reply)["error"]["code"],
        "unsupported_capability"
    );
    cleanup(&service, root);
}

#[test]
fn model_status_reads_claude_transcript_and_options() {
    let (mut service, root, herdr, path) = model_fixture_claude();
    let reply = model_request(&mut service, "model.status", json!({}));
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["agent"], "claude");
    assert_eq!(result["model"]["id"], "claude-opus-5-5");
    assert_eq!(result["model"]["label"], "Opus 5.5");
    assert_eq!(result["effort"], "medium");
    assert_eq!(
        result["models"],
        json!([
            {"id":"fable","label":"Fable"},
            {"id":"opus","label":"Opus","current":true},
            {"id":"sonnet","label":"Sonnet"},
            {"id":"haiku","label":"Haiku"}
        ])
    );
    assert_eq!(result["efforts"][0], json!({"id":"low","label":"Low"}));
    assert_eq!(result["efforts"].as_array().unwrap().len(), 5);
    assert_eq!(result["identity"]["native_session_id"], *SESSION);
    // A read never sends input.
    assert!(herdr.lock().unwrap().inputs.is_empty());

    // Fallback: a transcript without model records falls back to settings.json.
    fs::write(
        &path,
        format!(
            "{}\n",
            json!({"type":"user","uuid":"u1","sessionId":SESSION,
                   "message":{"role":"user","content":"hi"}})
        ),
    )
    .unwrap();
    fs::write(
        root.join("settings.json"),
        json!({"model":"sonnet","effortLevel":"high"}).to_string(),
    )
    .unwrap();
    let reply = model_request(&mut service, "model.status", json!({}));
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["model"]["id"], "sonnet");
    assert_eq!(result["model"]["label"], "Sonnet");
    assert_eq!(result["effort"], "high");
    assert_eq!(
        result["models"],
        json!([
            {"id":"fable","label":"Fable"},
            {"id":"opus","label":"Opus"},
            {"id":"sonnet","label":"Sonnet","current":true},
            {"id":"haiku","label":"Haiku"}
        ])
    );
    cleanup(&service, root);
}

#[test]
fn model_status_reads_codex_turn_context_and_catalog() {
    let (mut service, root, herdr, path) = model_fixture_codex();
    let reply = model_request(&mut service, "model.status", json!({}));
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["agent"], "codex");
    assert_eq!(result["model"]["id"], "gpt-6.1-sol");
    assert_eq!(result["model"]["label"], "GPT-6.1-Sol");
    assert_eq!(result["effort"], "low");
    assert_eq!(
        result["models"],
        json!([
            {"id":"gpt-6.1-sol","label":"GPT-6.1-Sol","current":true},
            {"id":"gpt-6-astra","label":"GPT-6-Astra"},
            {"id":"gpt-6-luna","label":"GPT-6-Luna"}
        ])
    );
    assert_eq!(
        result["efforts"],
        json!([
            {"id":"low","label":"Low"},
            {"id":"medium","label":"Medium"},
            {"id":"high","label":"High"},
            {"id":"xhigh","label":"Extra high"}
        ])
    );
    assert!(herdr.lock().unwrap().inputs.is_empty());
    // Without turn_context records, config.toml defaults apply.
    fs::write(
        &path,
        format!(
            "{}\n",
            json!({"type":"session_meta","payload":{"id":CODEX_THREAD,"cwd":"/fixture/work"}})
        ),
    )
    .unwrap();
    let reply = model_request(&mut service, "model.status", json!({}));
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["model"]["id"], "gpt-6.1-sol");
    assert_eq!(result["model"]["label"], "GPT-6.1-Sol");
    assert_eq!(result["effort"], "low");
    cleanup(&service, root);
}

#[test]
fn model_updated_emits_on_transcript_change_only() {
    let (mut service, root, _herdr, path) = model_fixture_claude();
    let mut out = Vec::new();
    service.poll(&mut out).unwrap();
    let events: Vec<Value> = frames(out)
        .into_iter()
        .filter(|frame| frame["event"] == "model.updated")
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["model"], "claude-opus-5-5");
    assert_eq!(events[0]["data"]["effort"], "medium");
    // Unchanged file: no further emission.
    let mut out = Vec::new();
    service.poll(&mut out).unwrap();
    assert!(
        !frames(out)
            .iter()
            .any(|frame| frame["event"] == "model.updated")
    );
    // A new assistant record with a different pair re-emits.
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!(
        "{}\n",
        json!({"type":"assistant","uuid":"am2","sessionId":SESSION,
               "message":{"model":"claude-sonnet-5-5","role":"assistant","content":"next"},
               "effort":"high"})
    ));
    fs::write(&path, text).unwrap();
    let mut out = Vec::new();
    service.poll(&mut out).unwrap();
    let events: Vec<Value> = frames(out)
        .into_iter()
        .filter(|frame| frame["event"] == "model.updated")
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["data"]["model"], "claude-sonnet-5-5");
    assert_eq!(events[0]["data"]["effort"], "high");
    // Without the opt-in no events are emitted.
    service.include_model_control = false;
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!(
        "{}\n",
        json!({"type":"assistant","uuid":"am3","sessionId":SESSION,
               "message":{"model":"claude-haiku-4-5","role":"assistant","content":"more"},
               "effort":"low"})
    ));
    fs::write(&path, text).unwrap();
    let mut out = Vec::new();
    service.poll(&mut out).unwrap();
    assert!(
        !frames(out)
            .iter()
            .any(|frame| frame["event"] == "model.updated")
    );
    cleanup(&service, root);
}

#[test]
fn model_set_claude_confirms_each_step_and_reports_partial() {
    let (mut service, root, herdr, _path) = model_fixture_claude();
    script_model(
        &herdr,
        vec![claude_model_screen("/model sonnet"), claude_effort_screen()],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"sonnet","effort":"high","expect":expect_pair("claude-opus-5-5", Some("medium"))}),
    );
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["confirmed"], true);
    assert_eq!(result["error"], Value::Null);
    assert_eq!(result["applied"]["model"]["id"], "sonnet");
    assert_eq!(result["applied"]["model"]["label"], "Sonnet 5.5");
    assert_eq!(result["applied"]["effort"], "high");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![prompt_input("/model sonnet"), prompt_input("/effort high")]
    );

    // Partial: the model step succeeds, the effort step is invalid.
    script_model(
        &herdr,
        vec![
            claude_model_screen("/model sonnet"),
            claude_invalid_effort_screen(),
        ],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"sonnet","effort":"high","expect":expect_pair("claude-opus-5-5", Some("medium"))}),
    );
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["confirmed"], false);
    assert_eq!(result["error"], "invalid_effort");
    assert_eq!(result["applied"]["model"]["label"], "Sonnet 5.5");
    assert_eq!(result["applied"]["effort"], "medium");
    cleanup(&service, root);
}

#[test]
fn model_set_codex_walks_picker_with_arrows_and_session_key() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    script_model(
        &herdr,
        vec![
            codex_model_list_screen(),
            codex_effort_list_screen(),
            codex_effort_list_screen(),
            codex_composer_toast("gpt-6-luna", "high"),
        ],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"gpt-6-luna","effort":"high","expect":expect_pair("gpt-6.1-sol", Some("low"))}),
    );
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["confirmed"], true);
    assert_eq!(result["applied"]["model"]["id"], "gpt-6-luna");
    assert_eq!(result["applied"]["model"]["label"], "GPT-6-Luna");
    assert_eq!(result["applied"]["effort"], "high");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            prompt_input("/model"),
            model_keys_input(&["4"]),
            model_keys_input(&["down"]),
            model_keys_input(&["s"]),
        ]
    );
    cleanup(&service, root);
}

#[test]
fn model_set_codex_advanced_reasoning_opens_nested_entry() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    script_model(
        &herdr,
        vec![
            codex_model_list_screen(),
            codex_effort_list_screen(),
            codex_effort_list_screen(),
            codex_effort_list_screen(),
            codex_effort_list_screen(),
            codex_advanced_screen(),
            codex_composer_toast("gpt-6-luna", "max"),
        ],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"gpt-6-luna","effort":"max","expect":expect_pair("gpt-6.1-sol", Some("low"))}),
    );
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["confirmed"], true);
    assert_eq!(result["applied"]["effort"], "max");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            prompt_input("/model"),
            model_keys_input(&["4"]),
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["enter"]),
            model_keys_input(&["s"]),
        ]
    );
    cleanup(&service, root);
}

#[test]
fn model_set_refuses_busy_stale_and_mismatched_pickers() {
    // Working agent: refuse before any input.
    let (mut service, root, herdr, _) = model_fixture_claude();
    herdr.lock().unwrap().status = Some("working".into());
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"sonnet","expect":expect_pair("claude-opus-5-5", Some("medium"))}),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "agent_busy");
    assert!(herdr.lock().unwrap().inputs.is_empty());

    // Stale expect: the transcript disagrees with the client's view.
    herdr.lock().unwrap().status = Some("idle".into());
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"sonnet","expect":expect_pair("claude-opus-4", Some("medium"))}),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "stale");
    assert!(herdr.lock().unwrap().inputs.is_empty());
    cleanup(&service, root);

    // Codex picker never opens: agent prompt sent, then escape recovery.
    let (mut service, root2, herdr2, _) = model_fixture_codex();
    herdr2.lock().unwrap().status = Some("idle".into());
    script_model(&herdr2, vec!["› Ask Codex to do anything".to_owned()]);
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"gpt-6-luna","expect":expect_pair("gpt-6.1-sol", Some("low"))}),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "picker_mismatch");
    assert_eq!(
        herdr2.lock().unwrap().inputs,
        vec![
            prompt_input("/model"),
            model_keys_input(&["esc"]),
            model_keys_input(&["esc"])
        ]
    );
    cleanup(&service, root2);
}

#[test]
fn model_set_validates_efforts_and_requires_watch() {
    let (mut service, root, _, _) = model_fixture_claude();
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"effort":"turbo","expect":expect_pair("claude-opus-5-5", Some("medium"))}),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "invalid_effort");
    let reply = model_request(&mut service, "model.set", json!({"model":"sonnet"}));
    assert_eq!(result_frame(&reply)["error"]["code"], "invalid_request");
    service.watch = None;
    let reply = model_request(&mut service, "model.status", json!({}));
    assert_eq!(result_frame(&reply)["error"]["code"], "not_watching");
    cleanup(&service, root);
}
