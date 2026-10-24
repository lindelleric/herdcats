// Included in service::tests to exercise model control against fake Herdr.

/// A Claude screen whose composer line (the last ❯ row) is the idle
/// placeholder, as the TUI shows once a submitted command has scrolled up.
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
           ⎿  Set model to Sonnet 5.5 and saved as your default for new sessions\n\
         ❯ Try \"fix lint errors\"\n"
    )
}

fn claude_effort_screen() -> String {
    "  Claude Code                         v2.1.309\n\
     ❯ /effort high\n\
       ⎿  Set effort level to high (saved as your default for new sessions)\n\
     ❯ Try \"fix lint errors\"\n"
        .to_owned()
}

fn claude_invalid_effort_screen() -> String {
    "  Claude Code                         v2.1.309\n\
     ❯ /effort high\n\
       ⎿  Invalid argument: high. Valid options are: low, medium, high, xhigh, max, auto\n\
     ❯ Try \"fix lint errors\"\n"
        .to_owned()
}

/// Model list with the cursor on the given one-based row.
fn codex_model_list_screen(cursor: usize) -> String {
    let rows = [
        (
            "1. GPT-6.1-Sol (current)",
            "Latest workhorse model for coding and everyday work.",
        ),
        (
            "2. GPT-6-Astra",
            "Frontier intelligence for the most demanding work.",
        ),
        ("3. GPT-6-Sol", "Previous generation workhorse model."),
        (
            "4. GPT-6-Luna",
            "Fast and affordable model for easier tasks.",
        ),
    ];
    let mut screen = String::from(
        "  To get started, describe a task or try one of these commands:\n\
         /model - choose what model and reasoning effort to use\n\
         Select Model and Effort\n",
    );
    for (index, (label, description)) in rows.iter().enumerate() {
        let marker = if index + 1 == cursor { "› " } else { "  " };
        screen.push_str(&format!("{marker}{label}  {description}\n"));
    }
    screen.push_str("enter select · esc back");
    screen
}

/// Effort screen for GPT-6-Luna with the cursor on the given one-based row.
fn codex_effort_list_screen(cursor: usize) -> String {
    let rows = [
        ("1. Low", "Fast responses with lighter reasoning"),
        (
            "2. Medium (default)",
            "Balances speed and reasoning depth for everyday tasks",
        ),
        ("3. High", "Greater reasoning depth for complex problems"),
        (
            "4. Extra high",
            "Extra high reasoning depth for complex problems",
        ),
        ("5. More reasoning…", "Max consumes usage limits faster"),
    ];
    let mut screen = String::from("Select Reasoning Level for GPT-6-Luna\n");
    for (index, (label, description)) in rows.iter().enumerate() {
        let marker = if index + 1 == cursor { "› " } else { "  " };
        screen.push_str(&format!("{marker}{label}  {description}\n"));
    }
    screen.push_str("enter default · s session · esc back");
    screen
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
    let label = model_control::codex_slug_label(slug);
    format!(
        "• Model changed to {slug} {effort} for this session only\n› Ask Codex to do anything\n{label} {effort} · Context 100% left"
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
    let efforts = json!([
        {"id":"low","label":"Low"},
        {"id":"medium","label":"Medium"},
        {"id":"high","label":"High"},
        {"id":"xhigh","label":"Extra high"},
        {"id":"max","label":"Max"}
    ]);
    assert_eq!(
        result["models"],
        json!([
            {"id":"default","label":"Default","efforts":efforts},
            {"id":"fable","label":"Fable 5.1","efforts":efforts},
            {"id":"opus","label":"Opus 5.5","current":true,"efforts":efforts},
            {"id":"sonnet","label":"Sonnet 5.5","efforts":efforts},
            {"id":"haiku","label":"Haiku 5.5","efforts":efforts}
        ])
    );
    assert_eq!(result["efforts"], efforts);
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
    assert_eq!(result["model"]["label"], "Sonnet 5.5");
    assert_eq!(result["effort"], "high");
    let models = result["models"].as_array().unwrap().clone();
    assert_eq!(
        models.iter().find(|entry| entry["id"] == "sonnet").unwrap()["current"],
        true
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
            {"id":"gpt-6.1-sol","label":"GPT-6.1-Sol","current":true,"efforts":[
                {"id":"low","label":"Low"},{"id":"medium","label":"Medium"},
                {"id":"high","label":"High"},{"id":"xhigh","label":"Extra high"}]},
            {"id":"gpt-6-astra","label":"GPT-6-Astra","efforts":[
                {"id":"low","label":"Low"},{"id":"medium","label":"Medium"},{"id":"high","label":"High"}]},
            {"id":"gpt-6-luna","label":"GPT-6-Luna","efforts":[
                {"id":"low","label":"Low"},{"id":"medium","label":"Medium"},
                {"id":"high","label":"High"},{"id":"xhigh","label":"Extra high"},{"id":"max","label":"Max"}]}
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
    // Without a footer or turn_context records, config.toml defaults apply.
    herdr.lock().unwrap().screen = "› Ask Codex to do anything".into();
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
    assert_eq!(events[0]["data"]["model"], json!({"id":"claude-opus-5-5","label":"Opus 5.5"}));
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
    assert_eq!(events[0]["data"]["model"], json!({"id":"claude-sonnet-5-5","label":"Sonnet 5.5"}));
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

    cleanup(&service, root);
    let (mut service, root, herdr, _path) = model_fixture_claude();
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
    assert!(service.watch.as_ref().unwrap().model_override.is_some());
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(result_frame(&status)["result"]["model"]["id"], "sonnet");
    cleanup(&service, root);
}

#[test]
fn model_set_claude_stops_before_effort_when_occupant_turns_busy() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    script_model(&herdr, vec![claude_model_screen("/model sonnet")]);
    // The pane turns working right after the /model submission.
    herdr.lock().unwrap().status_after_inputs = Some((1, "working".into()));
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"sonnet","effort":"high","expect":expect_pair("claude-opus-5-5", Some("medium"))}),
    );
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["confirmed"], false);
    assert_eq!(result["error"], "stale");
    assert_eq!(result["applied"]["model"]["label"], "Sonnet 5.5");
    assert_eq!(result["applied"]["effort"], "medium");
    // Only the confirmed model command was sent.
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![prompt_input("/model sonnet")]
    );
    cleanup(&service, root);
}

#[test]
fn model_set_refuses_draft_composers_without_sending() {
    // Claude: a half-written desk draft appends to any submitted prompt.
    let (mut service, root, herdr, _) = model_fixture_claude();
    herdr.lock().unwrap().screen = "  Claude Code\n❯ half-written draft\n".into();
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"sonnet","expect":expect_pair("claude-opus-5-5", Some("medium"))}),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "composer_not_empty");
    assert!(herdr.lock().unwrap().inputs.is_empty());
    cleanup(&service, root);

    // Codex: same guard before opening the picker.
    let (mut service, root, herdr, _) = model_fixture_codex();
    herdr.lock().unwrap().screen = "› hello draft\nGPT-6.1-Sol low".into();
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"gpt-6-luna","expect":expect_pair("gpt-6.1-sol", Some("low"))}),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "composer_not_empty");
    assert!(herdr.lock().unwrap().inputs.is_empty());
    cleanup(&service, root);
}

#[test]
fn model_set_codex_walks_picker_with_arrows_and_session_key() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    script_model(
        &herdr,
        vec![
            codex_model_list_screen(1),
            codex_model_list_screen(2),
            codex_model_list_screen(3),
            codex_model_list_screen(4),
            codex_effort_list_screen(1),
            codex_effort_list_screen(2),
            codex_effort_list_screen(3),
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
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["enter"]),
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["s"]),
        ]
    );
    cleanup(&service, root);
}

#[test]
fn model_set_codex_model_only_drops_an_effort_the_target_does_not_offer() {
    // Current effort xhigh; GPT-6-Astra offers only low/medium/high, so the
    // model switch keeps Astra's default row instead of failing on xhigh.
    let (mut service, root, herdr, path) = model_fixture_codex();
    let text = fs::read_to_string(&path).unwrap().replace(
        r#""effort":"low""#,
        r#""effort":"xhigh""#,
    );
    fs::write(&path, text).unwrap();
    herdr.lock().unwrap().screen =
        "› Ask Codex to do anything\nGPT-6.1-Sol Extra high · Context 100% left".into();
    let astra = |cursor| codex_effort_list_screen(cursor).replace("GPT-6-Luna", "GPT-6-Astra");
    script_model(
        &herdr,
        vec![
            codex_model_list_screen(1),
            codex_model_list_screen(2),
            astra(2),
            codex_composer_toast("gpt-6-astra", "medium"),
        ],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"gpt-6-astra","expect":expect_pair("gpt-6.1-sol", Some("xhigh"))}),
    );
    let result = result_frame(&reply)["result"].clone();
    assert_eq!(result["confirmed"], true, "{reply:?}");
    assert_eq!(result["applied"]["model"]["id"], "gpt-6-astra");
    assert_eq!(result["applied"]["effort"], "medium");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            prompt_input("/model"),
            model_keys_input(&["down"]),
            model_keys_input(&["enter"]),
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
            codex_model_list_screen(1),
            codex_model_list_screen(2),
            codex_model_list_screen(3),
            codex_model_list_screen(4),
            codex_effort_list_screen(1),
            codex_effort_list_screen(2),
            codex_effort_list_screen(3),
            codex_effort_list_screen(4),
            codex_effort_list_screen(5),
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
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["enter"]),
            model_keys_input(&["down"]),
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
fn model_set_codex_aborts_when_cursor_misses_the_target_row() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    // The fixture ignores the effort arrows: the cursor stays on Low while
    // High was targeted. `s` must never be sent.
    script_model(
        &herdr,
        vec![
            codex_model_list_screen(1),
            codex_model_list_screen(2),
            codex_model_list_screen(3),
            codex_model_list_screen(4),
            codex_effort_list_screen(1),
            codex_effort_list_screen(1),
        ],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"gpt-6-luna","effort":"high","expect":expect_pair("gpt-6.1-sol", Some("low"))}),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "picker_mismatch");
    let inputs = herdr.lock().unwrap().inputs.clone();
    assert_eq!(
        inputs,
        vec![
            prompt_input("/model"),
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["enter"]),
            model_keys_input(&["down"]),
            model_keys_input(&["down"]),
            model_keys_input(&["esc"]),
        ]
    );
    assert!(
        !inputs
            .iter()
            .any(|input| input["params"]["keys"] == json!(["s"]))
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

    // Codex picker never opens: never send Escape into the composer.
    let (mut service, root2, herdr2, _) = model_fixture_codex();
    herdr2.lock().unwrap().status = Some("idle".into());
    script_model(&herdr2, vec!["› Ask Codex to do anything".to_owned()]);
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"gpt-6-luna","expect":expect_pair("gpt-6.1-sol", Some("low"))}),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "picker_mismatch");
    assert_eq!(herdr2.lock().unwrap().inputs, vec![prompt_input("/model")]);
    cleanup(&service, root2);
}

#[test]
fn model_set_codex_midflow_busy_pane_dismisses_and_stales() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    // The pane turns working right after the /model submission, before any
    // keystroke batch.
    herdr.lock().unwrap().status_after_inputs = Some((1, "working".into()));
    script_model(&herdr, vec![codex_model_list_screen(1)]);
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"gpt-6-luna","effort":"high","expect":expect_pair("gpt-6.1-sol", Some("low"))}),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "stale");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![prompt_input("/model"), model_keys_input(&["esc"])]
    );
    cleanup(&service, root);
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

#[test]
fn model_set_claude_rechecks_before_effort_and_keeps_partial_model() {
    for occupant_changed in [false, true] {
        let (mut service, root, herdr, _) = model_fixture_claude();
        script_model(&herdr, vec![claude_model_screen("/model sonnet")]);
        if occupant_changed {
            herdr.lock().unwrap().terminal_after_input = Some("replacement".into());
        } else {
            herdr.lock().unwrap().status_after_inputs = Some((1, "working".into()));
        }
        let reply = model_request(
            &mut service,
            "model.set",
            json!({
                "model":"sonnet", "effort":"high",
                "expect":expect_pair("claude-opus-5-5", Some("medium"))
            }),
        );
        let result = &result_frame(&reply)["result"];
        assert_eq!(result["confirmed"], false);
        assert_eq!(result["error"], "stale");
        assert_eq!(result["applied"]["model"]["id"], "sonnet");
        assert_eq!(result["applied"]["effort"], "medium");
        assert_eq!(
            herdr.lock().unwrap().inputs,
            vec![prompt_input("/model sonnet")]
        );
        cleanup(&service, root);
    }
}

#[test]
fn model_set_codex_rechecks_between_arrow_sends() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    script_model(
        &herdr,
        vec![codex_model_list_screen(1), codex_model_list_screen(2)],
    );
    herdr.lock().unwrap().status_after_inputs = Some((2, "working".into()));
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"gpt-6-luna", "effort":"high",
            "expect":expect_pair("gpt-6.1-sol", Some("low"))
        }),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "stale");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            prompt_input("/model"),
            model_keys_input(&["down"]),
            model_keys_input(&["esc"])
        ]
    );
    cleanup(&service, root);
}

#[test]
fn model_cursor_verification_rejects_wrong_model_and_rows() {
    let (service, root, herdr, _) = model_fixture_codex();
    let expected = model_control::parse_codex_screen(&codex_effort_list_screen(3));
    for screen in [
        codex_effort_list_screen(3).replace("GPT-6-Luna", "GPT-6-Astra"),
        codex_effort_list_screen(3).replace("3. High", "3. Low"),
    ] {
        herdr.lock().unwrap().screen = screen;
        assert!(!service.verify_cursor("w1:p1", &expected, Some(2)));
    }
    cleanup(&service, root);
}

#[test]
fn model_status_keeps_empty_catalog_efforts() {
    let (mut service, root, _, _) = model_fixture_codex();
    fs::write(
        root.join("codex-home/models_cache.json"),
        json!({"models":[{
            "slug":"gpt-6.1-sol", "visibility":"list", "supported_reasoning_levels":[]
        }]})
        .to_string(),
    )
    .unwrap();
    let reply = model_request(&mut service, "model.status", json!({}));
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["models"][0]["efforts"], json!([]));
    assert_eq!(result["efforts"], json!([]));
    cleanup(&service, root);
}

#[test]
fn model_set_codex_selects_tenth_model_with_arrows() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    let render = |cursor| {
        let mut screen = String::from("Select Model and Effort\n");
        for row in 1..=10 {
            let label = if row == 10 {
                "GPT-6-Luna".to_owned()
            } else {
                format!("Model {row}")
            };
            screen.push_str(&format!(
                "{} {row}. {label}\n",
                if row == cursor { "›" } else { " " }
            ));
        }
        screen
    };
    let mut screens: Vec<String> = (1..=10).map(render).collect();
    screens.extend([
        codex_effort_list_screen(3),
        codex_composer_toast("gpt-6-luna", "high"),
    ]);
    script_model(&herdr, screens);
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"gpt-6-luna", "effort":"high",
            "expect":expect_pair("gpt-6.1-sol", Some("low"))
        }),
    );
    assert_eq!(result_frame(&reply)["result"]["confirmed"], true);
    let mut expected = vec![prompt_input("/model")];
    expected.extend((0..9).map(|_| model_keys_input(&["down"])));
    expected.extend([model_keys_input(&["enter"]), model_keys_input(&["s"])]);
    assert_eq!(herdr.lock().unwrap().inputs, expected);
    cleanup(&service, root);
}

#[test]
fn model_set_claude_preserves_model_when_draft_appears_before_effort() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    script_model(
        &herdr,
        vec![
            claude_model_screen("/model sonnet")
                .replace("Try \"fix lint errors\"", "new desktop draft"),
        ],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"sonnet", "effort":"high",
            "expect":expect_pair("claude-opus-5-5", Some("medium"))
        }),
    );
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["confirmed"], false);
    assert_eq!(result["error"], "composer_not_empty");
    assert_eq!(result["applied"]["model"]["id"], "sonnet");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![prompt_input("/model sonnet")]
    );
    cleanup(&service, root);
}

#[test]
fn model_override_preserves_applied_until_scanned_pair_changes() {
    let original = Some(("old-model".into(), Some("low".into())));
    let applied = Some(("new-model".into(), Some("high".into())));
    let mut slot = Some(ModelOverride {
        applied: applied.clone(),
        applied_label: None,
        scanned_at_set: original.clone(),
    });
    assert_eq!(ModelOverride::resolve(&mut slot, original.clone()), applied);
    assert!(slot.is_some());
    let next = Some(("new-model".into(), Some("high".into())));
    assert_eq!(ModelOverride::resolve(&mut slot, next.clone()), next);
    assert!(slot.is_none());
    let mut slot = Some(ModelOverride {
        applied,
        applied_label: None,
        scanned_at_set: original,
    });
    let desk = Some(("desk-model".into(), Some("medium".into())));
    assert_eq!(ModelOverride::resolve(&mut slot, desk.clone()), desk);
    assert!(slot.is_none());
}

#[test]
fn model_set_claude_then_effort_uses_override_and_status_reports_applied() {
    let (mut service, root, herdr, path) = model_fixture_claude();
    script_model(
        &herdr,
        vec![claude_model_screen("/model sonnet"), claude_effort_screen()],
    );
    let first = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"sonnet", "expect":expect_pair("claude-opus-5-5", Some("medium"))
        }),
    );
    assert_eq!(result_frame(&first)["result"]["confirmed"], true);
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(result_frame(&status)["result"]["model"]["id"], "sonnet");
    let second = model_request(
        &mut service,
        "model.set",
        json!({
            "effort":"high", "expect":expect_pair("sonnet", Some("medium"))
        }),
    );
    assert_eq!(result_frame(&second)["result"]["confirmed"], true);
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(result_frame(&status)["result"]["effort"], "high");
    assert!(service.watch.as_ref().unwrap().model_override.is_some());
    // A fresh assistant record wins, including the concrete Claude id.
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!("{}\n", json!({"type":"assistant","uuid":"next-turn","sessionId":SESSION,
        "message":{"model":"claude-sonnet-5-5","role":"assistant","content":"next"},"effort":"high"})));
    fs::write(&path, text).unwrap();
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(
        result_frame(&status)["result"]["model"]["id"],
        "claude-sonnet-5-5"
    );
    assert!(service.watch.as_ref().unwrap().model_override.is_none());
    cleanup(&service, root);
}

#[test]
fn model_set_codex_then_effort_uses_override_until_desk_changes_transcript() {
    let (mut service, root, herdr, path) = model_fixture_codex();
    script_model(
        &herdr,
        vec![
            codex_model_list_screen(1),
            codex_model_list_screen(2),
            codex_model_list_screen(3),
            codex_model_list_screen(4),
            codex_effort_list_screen(1),
            codex_composer_toast("gpt-6-luna", "low"),
            codex_model_list_screen(4),
            codex_effort_list_screen(1),
            codex_effort_list_screen(2),
            codex_effort_list_screen(3),
            codex_composer_toast("gpt-6-luna", "high"),
        ],
    );
    let first = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"gpt-6-luna", "expect":expect_pair("gpt-6.1-sol", Some("low"))
        }),
    );
    assert_eq!(result_frame(&first)["result"]["confirmed"], true);
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(result_frame(&status)["result"]["model"]["id"], "gpt-6-luna");
    let second = model_request(
        &mut service,
        "model.set",
        json!({
            "effort":"high", "expect":expect_pair("gpt-6-luna", Some("low"))
        }),
    );
    assert_eq!(result_frame(&second)["result"]["confirmed"], true);
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(result_frame(&status)["result"]["effort"], "high");
    // With no parseable live footer, a later transcript record becomes authoritative.
    herdr.lock().unwrap().screen = "› Ask Codex to do anything".into();
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str(&format!(
        "{}\n",
        json!({"type":"turn_context","payload":{
        "model":"gpt-6-astra","effort":"medium","cwd":"/fixture/work"}})
    ));
    fs::write(&path, text).unwrap();
    let mut out = Vec::new();
    service.poll(&mut out).unwrap();
    assert!(
        frames(out)
            .iter()
            .any(|frame| frame["event"] == "model.updated"
                && frame["data"]["model"]["id"] == "gpt-6-astra")
    );
    assert!(service.watch.as_ref().unwrap().model_override.is_none());
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(
        result_frame(&status)["result"]["model"]["id"],
        "gpt-6-astra"
    );
    cleanup(&service, root);
}

#[test]
fn model_override_resets_with_occupant() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    script_model(&herdr, vec![claude_model_screen("/model sonnet")]);
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"sonnet", "expect":expect_pair("claude-opus-5-5", Some("medium"))
        }),
    );
    assert_eq!(result_frame(&reply)["result"]["confirmed"], true);
    assert!(service.watch.as_ref().unwrap().model_override.is_some());
    herdr.lock().unwrap().terminal_id = Some("replacement".into());
    service.poll(&mut Vec::new()).unwrap();
    assert!(service.watch.as_ref().unwrap().model_override.is_none());
    cleanup(&service, root);
}

#[test]
fn model_set_codex_waits_for_delayed_cursor_on_real_picker_screens() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    let models = include_str!("fixtures/codex-model-list.txt").to_owned();
    let before = include_str!("fixtures/codex-effort-list.txt").to_owned();
    let after = before
        .replacen("› 1.", "  1.", 1)
        .replacen("  2. Medium", "› 2. Medium", 1);
    script_model(
        &herdr,
        vec![
            models,
            before.clone(),
            after.clone(),
            codex_composer_toast("gpt-6.1-sol", "medium"),
        ],
    );
    // After Down, pane.read first sees the old render, then the moved cursor.
    herdr.lock().unwrap().screens_after_input[2] = [before, after].into();
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "effort":"medium", "expect":expect_pair("gpt-6.1-sol", Some("low"))
        }),
    );
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["confirmed"], true);
    assert_eq!(result["applied"]["effort"], "medium");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            prompt_input("/model"),
            model_keys_input(&["enter"]),
            model_keys_input(&["down"]),
            model_keys_input(&["s"])
        ]
    );
    cleanup(&service, root);
}

#[test]
fn model_cursor_verification_ignores_changed_status_suffix_order() {
    let (service, root, herdr, _) = model_fixture_codex();
    let before = include_str!("fixtures/codex-effort-list.txt");
    let expected = model_control::parse_codex_screen(before);
    herdr.lock().unwrap().screen = before.replace("(default) (current)", "(current) (default)");
    assert!(service.verify_cursor("w1:p1", &expected, Some(0)));
    cleanup(&service, root);
}

#[test]
fn model_set_codex_confirms_last_toast_after_an_older_change() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    let old = "• Model changed to gpt-6.1-sol medium for this session only\n";
    let mut screens = vec![
        codex_model_list_screen(1),
        codex_model_list_screen(2),
        codex_model_list_screen(3),
        codex_model_list_screen(4),
        codex_effort_list_screen(1),
        codex_effort_list_screen(2),
    ]
    .into_iter()
    .map(|screen| format!("{old}{screen}"))
    .collect::<Vec<_>>();
    screens.push(format!(
        "{old}{}",
        codex_composer_toast("gpt-6-luna", "medium")
    ));
    script_model(&herdr, screens);
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"gpt-6-luna", "effort":"medium",
            "expect":expect_pair("gpt-6.1-sol", Some("low"))
        }),
    );
    assert_eq!(result_frame(&reply)["result"]["confirmed"], true);
    assert_eq!(
        result_frame(&reply)["result"]["applied"]["model"]["id"],
        "gpt-6-luna"
    );
    assert!(
        !herdr
            .lock()
            .unwrap()
            .inputs
            .iter()
            .any(|input| input["params"]["keys"] == json!(["esc"]))
    );
    cleanup(&service, root);
}

#[test]
fn model_set_codex_leftover_identical_toast_times_out_without_escape_into_composer() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    let toast = codex_composer_toast("gpt-6.1-sol", "medium");
    let models = include_str!("fixtures/codex-model-list.txt");
    let effort = include_str!("fixtures/codex-effort-list.txt");
    let moved = effort
        .replacen("› 1.", "  1.", 1)
        .replacen("  2. Medium", "› 2. Medium", 1);
    script_model(
        &herdr,
        vec![
            format!("{toast}\n{models}"),
            format!("{toast}\n{effort}"),
            format!("{toast}\n{moved}"),
            toast,
        ],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "effort":"medium", "expect":expect_pair("gpt-6.1-sol", Some("low"))
        }),
    );
    assert_eq!(result_frame(&reply)["error"]["code"], "confirm_timeout");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            prompt_input("/model"),
            model_keys_input(&["enter"]),
            model_keys_input(&["down"]),
            model_keys_input(&["s"])
        ]
    );
    cleanup(&service, root);
}

#[test]
fn codex_dismissal_waits_for_nested_picker_transitions_and_stops_at_composer() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    let advanced = codex_advanced_screen();
    let effort = codex_effort_list_screen(5);
    script_model(
        &herdr,
        vec![effort.clone(), "› Ask Codex to do anything".into()],
    );
    {
        let mut state = herdr.lock().unwrap();
        state.screen = advanced.clone();
        // First Escape initially returns the stale Advanced screen.
        state.screens_after_input[0] = [advanced, effort].into();
    }
    let identity = watched_identity(&mut service);
    service.dismiss_codex_picker(&identity);
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![model_keys_input(&["esc"]), model_keys_input(&["esc"])]
    );
    assert_eq!(herdr.lock().unwrap().screen, "› Ask Codex to do anything");
    cleanup(&service, root);
}

/// The identity the watch verifies for the fixture's current occupant.
fn watched_identity(service: &mut Service) -> Identity {
    service.poll(&mut Vec::new()).unwrap();
    service.watch.as_ref().unwrap().identity.clone().unwrap()
}

#[test]
fn model_status_lists_an_older_dated_claude_id_as_its_own_current_row() {
    let (mut service, root, _, path) = model_fixture_claude();
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("claude-opus-5-5", "claude-haiku-4-5-20251001");
    fs::write(&path, text).unwrap();
    let reply = model_request(&mut service, "model.status", json!({}));
    let models = result_frame(&reply)["result"]["models"].as_array().unwrap().clone();
    let haiku = models.iter().find(|m| m["id"] == "haiku").unwrap();
    assert_eq!(haiku["label"], "Haiku 5.5");
    assert!(haiku.get("current").is_none(), "{models:?}");
    let current: Vec<_> = models.iter().filter(|m| m["current"] == true).collect();
    assert_eq!(current.len(), 1);
    assert_eq!(current[0]["id"], "claude-haiku-4-5-20251001");
    assert_eq!(current[0]["label"], "Haiku 4.5");
    cleanup(&service, root);
}

#[test]
fn model_status_finds_a_latest_record_larger_than_the_tail_window() {
    let (mut service, root, _, path) = model_fixture_claude();
    let mut text = fs::read_to_string(&path).unwrap();
    // One assistant reply over 4 MiB, running a different model.
    text.push_str(&json!({"type":"assistant","uuid":"big","sessionId":SESSION,
        "message":{"model":"claude-sonnet-5-5","role":"assistant","content":"x".repeat(5 * 1024 * 1024)},
        "effort":"high"}).to_string());
    text.push('\n');
    fs::write(&path, text).unwrap();
    let reply = model_request(&mut service, "model.status", json!({}));
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["model"]["id"], "claude-sonnet-5-5", "{result:?}");
    assert_eq!(result["effort"], "high");
    cleanup(&service, root);
}

#[test]
fn model_status_reports_claudes_default_model_and_its_effort_without_a_configured_model() {
    let (mut service, root, _, path) = model_fixture_claude();
    // A fresh session: no assistant records yet.
    let text: String = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .filter(|line| !line.contains("claude-opus-5-5"))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(&path, text).unwrap();
    // No settings.json at all (a fresh install) still means Claude's default.
    let _ = fs::remove_file(root.join("settings.json"));
    let reply = model_request(&mut service, "model.status", json!({}));
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["model"], json!({"id":"default","label":"Default"}), "{result:?}");
    assert_eq!(result["effort"], Value::Null);
    fs::write(root.join("settings.json"), json!({"effortLevel":"high"}).to_string()).unwrap();
    let reply = model_request(&mut service, "model.status", json!({}));
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["model"], json!({"id":"default","label":"Default"}), "{result:?}");
    assert_eq!(result["effort"], "high");
    cleanup(&service, root);
}

#[test]
fn model_set_rechecks_the_occupant_right_before_the_first_prompt() {
    // The pane is replaced after the composer read; nothing may be sent.
    for agent in ["claude", "codex"] {
        let (mut service, root, herdr, _) = if agent == "claude" {
            model_fixture_claude()
        } else {
            model_fixture_codex()
        };
        // Swap the pane right after the composer read (Codex reads its footer
        // first), so only the recheck before the prompt can catch it.
        let composer_read = if agent == "claude" { 1 } else { 2 };
        herdr.lock().unwrap().terminal_after_reads = Some((composer_read, "replacement".into()));
        let expect = if agent == "claude" {
            expect_pair("claude-opus-5-5", Some("medium"))
        } else {
            expect_pair("gpt-6.1-sol", Some("low"))
        };
        let reply = model_request(&mut service, "model.set", json!({"effort":"high","expect":expect}));
        let frame = result_frame(&reply);
        let code = frame["error"]["code"].as_str().or(frame["result"]["error"].as_str());
        assert_eq!(code, Some("stale"), "{agent}: {reply:?}");
        assert!(herdr.lock().unwrap().inputs.is_empty(), "{agent}");
        cleanup(&service, root);
    }
}

#[test]
fn model_set_codex_reselecting_the_current_model_sends_nothing() {
    let (mut service, root, herdr, path) = model_fixture_codex();
    // An effort the picker has no row for (minimal) must survive a reselect.
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace(r#""effort":"low""#, r#""effort":"minimal""#);
    fs::write(&path, text).unwrap();
    herdr.lock().unwrap().screen = "› Ask Codex to do anything".into();
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"gpt-6.1-sol","expect":expect_pair("gpt-6.1-sol", Some("minimal"))}),
    );
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["confirmed"], true, "{reply:?}");
    assert_eq!(result["applied"]["effort"], "minimal");
    assert!(herdr.lock().unwrap().inputs.is_empty());
    cleanup(&service, root);
}

#[test]
fn model_set_codex_reselecting_the_current_effort_sends_nothing() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"effort":"low","expect":expect_pair("gpt-6.1-sol", Some("low"))}),
    );
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["confirmed"], true, "{reply:?}");
    assert_eq!(result["applied"]["effort"], "low");
    assert!(herdr.lock().unwrap().inputs.is_empty());
    cleanup(&service, root);
}

#[test]
fn model_set_claude_treats_the_active_alias_as_unchanged() {
    // The session runs claude-opus-5-5; the checked "opus" row plus a new
    // effort sends only /effort, never /model.
    let (mut service, root, herdr, _) = model_fixture_claude();
    script_model(&herdr, vec![claude_effort_screen()]);
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"opus","effort":"high","expect":expect_pair("claude-opus-5-5", Some("medium"))}),
    );
    assert_eq!(result_frame(&reply)["result"]["confirmed"], true, "{reply:?}");
    assert_eq!(herdr.lock().unwrap().inputs, vec![prompt_input("/effort high")]);
    cleanup(&service, root);
}

#[test]
fn model_set_refuses_a_typed_draft_that_matches_a_placeholder() {
    for (fixture, screen) in [
        ("claude", "❯ Try \"fix lint errors\""),
        ("codex", "› Explain this codebase"),
    ] {
        let (mut service, root, herdr, _) = if fixture == "claude" {
            model_fixture_claude()
        } else {
            model_fixture_codex()
        };
        {
            let mut state = herdr.lock().unwrap();
            state.screen = screen.into();
            // Typed by the user: same words, normal intensity.
            state.ansi_screen = Some(format!("\u{1b}[0m{screen}\u{1b}[0m"));
        }
        let expect = if fixture == "claude" {
            expect_pair("claude-opus-5-5", Some("medium"))
        } else {
            expect_pair("gpt-6.1-sol", Some("low"))
        };
        let reply = model_request(
            &mut service,
            "model.set",
            json!({"effort":"high","expect":expect}),
        );
        let frame = result_frame(&reply);
        let code = frame["error"]["code"].as_str().or(frame["result"]["error"].as_str());
        assert_eq!(code, Some("composer_not_empty"), "{fixture}: {reply:?}");
        assert!(herdr.lock().unwrap().inputs.is_empty(), "{fixture}");
        cleanup(&service, root);
    }
}

#[test]
fn codex_dismissal_never_sends_escape_into_a_replacement_occupant() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    let identity = watched_identity(&mut service);
    {
        let mut state = herdr.lock().unwrap();
        // Same pane id, new terminal that also has a picker open.
        state.terminal_id = Some("replacement".into());
        state.screen = codex_effort_list_screen(1);
    }
    service.dismiss_codex_picker(&identity);
    assert!(herdr.lock().unwrap().inputs.is_empty());
    cleanup(&service, root);
}

#[test]
fn model_set_claude_uses_its_own_confirmation_not_a_newer_desk_one() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    // Our /model haiku is followed on screen by a desk-side /model sonnet.
    let screen = "  Claude Code                         v2.1.309\n\
         ❯ /model haiku\n\
           ⎿  Set model to Haiku 5.5 and saved as your default for new sessions\n\
         ❯ /model sonnet\n\
           ⎿  Set model to Sonnet 5.5 and saved as your default for new sessions\n\
         ❯ Try \"fix lint errors\"\n"
        .to_owned();
    script_model(&herdr, vec![screen]);
    let reply = model_request(
        &mut service,
        "model.set",
        json!({"model":"haiku", "expect":expect_pair("claude-opus-5-5", Some("medium"))}),
    );
    // Our confirmation comes first after our echo; the desk's later
    // /model sonnet line is ignored.
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["confirmed"], true, "{reply:?}");
    assert_eq!(result["applied"]["model"], json!({"id":"haiku","label":"Haiku 5.5"}));
    cleanup(&service, root);
}

#[test]
fn codex_dismissal_never_sends_escape_to_unknown_or_composer_screens() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    for screen in [
        "Browsing transcript · ↵ rewind",
        "unrecognized screen",
        "› Ask Codex to do anything",
    ] {
        herdr.lock().unwrap().screen = screen.into();
        let identity = watched_identity(&mut service);
    service.dismiss_codex_picker(&identity);
    }
    assert!(herdr.lock().unwrap().inputs.is_empty());
    cleanup(&service, root);
}

#[test]
fn codex_dismissal_does_not_repeat_escape_on_an_unchanged_picker() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    script_model(&herdr, vec![]);
    herdr.lock().unwrap().screen = codex_effort_list_screen(1);
    let identity = watched_identity(&mut service);
    service.dismiss_codex_picker(&identity);
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![model_keys_input(&["esc"])]
    );
    cleanup(&service, root);
}

#[test]
fn model_set_claude_accepts_each_cache_switch_dialog_and_confirms_results() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    let model_dialog = include_str!("fixtures/claude-model-confirm.txt").to_owned();
    let effort_dialog =
        include_str!("fixtures/claude-effort-confirm.txt").replace("medium", "high");
    let applied_model = claude_model_screen("/model haiku").replace("Sonnet 5.5", "Haiku 5.5");
    script_model(
        &herdr,
        vec![
            model_dialog,
            applied_model,
            effort_dialog,
            claude_effort_screen(),
        ],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"haiku", "effort":"high", "expect":expect_pair("claude-opus-5-5", Some("medium"))
        }),
    );
    let result = &result_frame(&reply)["result"];
    assert_eq!(result["confirmed"], true);
    assert_eq!(result["applied"]["model"]["label"], "Haiku 5.5");
    assert_eq!(result["applied"]["effort"], "high");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![
            prompt_input("/model haiku"),
            model_keys_input(&["1"]),
            prompt_input("/effort high"),
            model_keys_input(&["1"])
        ]
    );
    cleanup(&service, root);
}

#[test]
fn model_set_claude_declines_mismatched_dialog_target() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    script_model(
        &herdr,
        vec![include_str!("fixtures/claude-effort-confirm.txt").into()],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "effort":"high", "expect":expect_pair("claude-opus-5-5", Some("medium"))
        }),
    );
    assert_eq!(result_frame(&reply)["result"]["error"], "picker_mismatch");
    assert_eq!(result_frame(&reply)["result"]["confirmed"], false);
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![prompt_input("/effort high"), model_keys_input(&["2"])]
    );
    cleanup(&service, root);
}

#[test]
fn model_set_claude_timeout_declines_only_a_still_visible_dialog() {
    for visible in [true, false] {
        let (mut service, root, herdr, _) = model_fixture_claude();
        let dialog = include_str!("fixtures/claude-effort-confirm.txt").replace("medium", "high");
        let after = if visible {
            dialog.clone()
        } else {
            claude_idle_screen()
        };
        script_model(&herdr, vec![dialog, after]);
        let reply = model_request(
            &mut service,
            "model.set",
            json!({
                "effort":"high", "expect":expect_pair("claude-opus-5-5", Some("medium"))
            }),
        );
        assert_eq!(result_frame(&reply)["result"]["error"], "confirm_timeout");
        let mut expected = vec![prompt_input("/effort high"), model_keys_input(&["1"])];
        if visible {
            expected.push(model_keys_input(&["2"]));
        }
        assert_eq!(herdr.lock().unwrap().inputs, expected);
        cleanup(&service, root);
    }
}

#[test]
fn model_set_claude_kept_result_is_failure_without_further_keys() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    script_model(
        &herdr,
        vec!["❯ /effort high\n  ⎿ Kept effort level as medium\n❯ Try \"fix lint errors\"".into()],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "effort":"high", "expect":expect_pair("claude-opus-5-5", Some("medium"))
        }),
    );
    assert_eq!(result_frame(&reply)["result"]["error"], "change_declined");
    assert_eq!(
        result_frame(&reply)["result"]["applied"]["effort"],
        "medium"
    );
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![prompt_input("/effort high")]
    );
    cleanup(&service, root);
}

#[test]
fn model_set_claude_dialog_acceptance_rechecks_idle_status() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    let dialog = include_str!("fixtures/claude-effort-confirm.txt").replace("medium", "high");
    script_model(&herdr, vec![dialog]);
    herdr.lock().unwrap().status_after_inputs = Some((1, "working".into()));
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "effort":"high", "expect":expect_pair("claude-opus-5-5", Some("medium"))
        }),
    );
    assert_eq!(result_frame(&reply)["result"]["error"], "stale");
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![prompt_input("/effort high")]
    );
    cleanup(&service, root);
}

#[test]
fn model_set_claude_declines_mismatched_model_label() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    script_model(
        &herdr,
        vec![include_str!("fixtures/claude-model-confirm.txt").into()],
    );
    let reply = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"sonnet", "expect":expect_pair("claude-opus-5-5", Some("medium"))
        }),
    );
    assert_eq!(result_frame(&reply)["result"]["error"], "picker_mismatch");
    assert_eq!(
        result_frame(&reply)["result"]["applied"]["model"]["id"],
        "claude-opus-5-5"
    );
    assert_eq!(
        herdr.lock().unwrap().inputs,
        vec![prompt_input("/model sonnet"), model_keys_input(&["2"])]
    );
    cleanup(&service, root);
}

#[test]
fn claude_override_retains_confirmed_label_in_status_events_and_later_effort_change() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    script_model(
        &herdr,
        vec![
            claude_model_screen("/model haiku").replace("Sonnet 5.5", "Haiku 5.5"),
            claude_effort_screen(),
        ],
    );
    let first = model_request(
        &mut service,
        "model.set",
        json!({
            "model":"haiku", "expect":expect_pair("claude-opus-5-5", Some("medium"))
        }),
    );
    assert_eq!(result_frame(&first)["result"]["confirmed"], true);
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(
        result_frame(&status)["result"]["model"],
        json!({"id":"haiku","label":"Haiku 5.5"})
    );
    // model.set polls after writing its result, so its frames already contain
    // the labeled event. The later status poll must not repeat that event.
    let updates: Vec<_> = first
        .iter()
        .filter(|frame| {
            frame["event"] == "model.updated"
                && frame["data"]["model"] == json!({"id":"haiku","label":"Haiku 5.5"})
        })
        .collect();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0]["data"]["effort"], "medium");
    assert!(!status.iter().any(|frame| frame["event"] == "model.updated"));
    let second = model_request(
        &mut service,
        "model.set",
        json!({
            "effort":"high", "expect":expect_pair("haiku", Some("medium"))
        }),
    );
    assert_eq!(
        result_frame(&second)["result"]["applied"]["model"]["label"],
        "Haiku 5.5"
    );
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(
        result_frame(&status)["result"]["model"]["label"],
        "Haiku 5.5"
    );
    cleanup(&service, root);
}

#[test]
fn codex_reconnected_bridge_uses_live_footer_for_status_expect_and_updates() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    herdr.lock().unwrap().screen =
        "› Ask Codex to do anything\nGPT-6-Luna high · Context 96% left · weekly 94% left".into();
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(result_frame(&status)["result"]["model"]["id"], "gpt-6-luna");
    assert_eq!(result_frame(&status)["result"]["effort"], "high");
    assert!(service.watch.as_ref().unwrap().model_override.is_none());
    // The unchanged transcript stamp must not suppress later footer changes.
    herdr.lock().unwrap().status = Some("done".into());
    herdr.lock().unwrap().screen =
        "› Ask Codex to do anything\nGPT-6-Luna Extra high · Context 96% left".into();
    let status = model_request(&mut service, "model.status", json!({}));
    assert!(
        status
            .iter()
            .any(|frame| frame["event"] == "model.updated" && frame["data"]["effort"] == "xhigh")
    );
    script_model(
        &herdr,
        vec![
            codex_model_list_screen(4),
            codex_effort_list_screen(4),
            codex_effort_list_screen(3),
            codex_composer_toast("gpt-6-luna", "high"),
        ],
    );
    let change = model_request(
        &mut service,
        "model.set",
        json!({
            "effort":"high", "expect":expect_pair("gpt-6-luna", Some("xhigh"))
        }),
    );
    assert_eq!(result_frame(&change)["result"]["confirmed"], true);
    cleanup(&service, root);
}

#[test]
fn codex_footer_falls_back_for_unknown_labels_missing_footers_and_busy_panes() {
    let (mut service, root, herdr, _) = model_fixture_codex();
    for (screen, status) in [
        (
            include_str!("fixtures/codex-idle-footer-unknown.txt"),
            "idle",
        ),
        ("› Ask Codex to do anything", "idle"),
        ("GPT-6-Luna high · Context 96% left", "working"),
        ("GPT-6-Luna high · Context 96% left", "blocked"),
    ] {
        {
            let mut state = herdr.lock().unwrap();
            state.screen = screen.into();
            state.status = Some(status.into());
        }
        let reply = model_request(&mut service, "model.status", json!({}));
        assert_eq!(result_frame(&reply)["result"]["model"]["id"], "gpt-6.1-sol");
        assert_eq!(result_frame(&reply)["result"]["effort"], "low");
    }
    cleanup(&service, root);
}

#[test]
fn claude_scan_never_uses_a_codex_footer() {
    let (mut service, root, herdr, _) = model_fixture_claude();
    herdr.lock().unwrap().screen = include_str!("fixtures/codex-idle-footer.txt").into();
    let status = model_request(&mut service, "model.status", json!({}));
    assert_eq!(
        result_frame(&status)["result"]["model"]["id"],
        "claude-opus-5-5"
    );
    cleanup(&service, root);
}

#[test]
fn model_override_keeps_confirmed_label_only_while_the_baseline_holds() {
    let baseline = Some(("claude-opus-5-5".into(), Some("medium".into())));
    let applied = Some(("haiku".into(), Some("medium".into())));
    let mut slot = Some(ModelOverride {
        applied: applied.clone(),
        applied_label: Some("Haiku 5.5".into()),
        scanned_at_set: baseline.clone(),
    });
    let observed = ModelOverride::resolve(&mut slot, baseline);
    assert_eq!(
        slot.as_ref().unwrap().label_for(&observed),
        Some("Haiku 5.5".into())
    );
    let actual = Some(("claude-haiku-5-5".into(), Some("medium".into())));
    assert_eq!(ModelOverride::resolve(&mut slot, actual.clone()), actual);
    assert!(slot.is_none());
}
