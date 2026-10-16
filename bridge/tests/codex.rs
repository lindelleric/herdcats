use herdcats_bridge::{
    adapter::Adapter,
    codex::{CodexAdapter, CodexResolver},
    follower::{AgentType, Follower, MAX_RECORD},
    herdr::{Pane, SessionRef},
};
use serde_json::json;
use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const THREAD_ID: &str = "f7b6ed8d-54fd-5ac3-bf0b-25eaa0e19912";
const ROLLOUT_FIXTURE: &str =
    include_str!("../../compatibility/phase1/fixtures/codex-rollout.jsonl");
const PENDING_FIXTURE: &str =
    include_str!("../../compatibility/phase1/fixtures/codex-pending-rollout.jsonl");
const QA_FIXTURE: &str =
    include_str!("../../compatibility/phase1/fixtures/codex-question-answer-rollout.jsonl");

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hc-codex-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path.canonicalize().unwrap())
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_pane(
    thread_id: Option<&str>,
    source: &str,
    agent: &str,
    session_agent: &str,
    kind: &str,
    cwd: &str,
) -> Pane {
    Pane {
        pane_id: "w1:p1".to_string(),
        terminal_id: "t1".to_string(),
        agent: Some(agent.to_string()),
        agent_status: "idle".to_string(),
        agent_session: thread_id.map(|id| SessionRef {
            agent: session_agent.to_string(),
            source: source.to_string(),
            kind: kind.to_string(),
            value: id.to_string(),
        }),
        foreground_cwd: Some(cwd.to_string()),
        cwd: Some(cwd.to_string()),
    }
}

#[test]
fn follower_parses_phase1_codex_fixtures() {
    // 1. Full rollout fixture
    let mut follower = Follower::for_agent(AgentType::Codex);
    let temp = Temp::new();
    let file_path = temp.0.join("rollout.jsonl");
    fs::write(&file_path, ROLLOUT_FIXTURE).unwrap();
    let file = File::open(&file_path).unwrap();

    let updated = follower.refresh_file(file, THREAD_ID).unwrap();
    assert!(updated);
    let transcript = &follower.transcript;
    assert_eq!(transcript.unknown_records, 0);

    let messages: Vec<_> = transcript
        .entries
        .iter()
        .filter(|e| e.kind == "message")
        .collect();
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role.as_deref(), Some("user"));
    assert_eq!(messages[1].role.as_deref(), Some("user"));
    assert!(
        messages[1]
            .text
            .contains("Run exactly this shell command: echo herdcats-probe > probe2.txt")
    );
    assert_eq!(messages[2].role.as_deref(), Some("assistant"));
    assert_eq!(messages[3].role.as_deref(), Some("user"));

    let tools: Vec<_> = transcript
        .entries
        .iter()
        .filter(|e| e.kind == "tool")
        .collect();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].tool_name.as_deref(), Some("exec"));
    assert_eq!(tools[0].status.as_deref(), Some("completed"));
    assert!(tools[0].text.contains("Fixture text"));
    assert_eq!(
        tools[1].tool_name.as_deref(),
        Some("request_user_input_async")
    );
    assert_eq!(tools[1].status.as_deref(), Some("completed"));

    // 2. Pending rollout fixture (tool call without output is in running status)
    let mut follower_pending = Follower::for_agent(AgentType::Codex);
    let file_path_pending = temp.0.join("pending.jsonl");
    fs::write(&file_path_pending, PENDING_FIXTURE).unwrap();
    let file_pending = File::open(&file_path_pending).unwrap();

    let updated_pending = follower_pending
        .refresh_file(file_pending, THREAD_ID)
        .unwrap();
    assert!(updated_pending);
    let transcript_pending = &follower_pending.transcript;
    let pending_tools: Vec<_> = transcript_pending
        .entries
        .iter()
        .filter(|e| e.kind == "tool")
        .collect();
    assert_eq!(pending_tools.len(), 1);
    assert_eq!(pending_tools[0].tool_name.as_deref(), Some("exec"));
    assert_eq!(pending_tools[0].status.as_deref(), Some("running"));

    // 3. Question-answer rollout fixture (function_call and function_call_output)
    let qa_thread_id = "5f864d7a-0e47-526a-84c3-d03c78582686";
    let mut follower_qa = Follower::for_agent(AgentType::Codex);
    let file_path_qa = temp.0.join("qa.jsonl");
    fs::write(&file_path_qa, QA_FIXTURE).unwrap();
    let file_qa = File::open(&file_path_qa).unwrap();

    let updated_qa = follower_qa.refresh_file(file_qa, qa_thread_id).unwrap();
    assert!(updated_qa);
    let transcript_qa = &follower_qa.transcript;
    let qa_tools: Vec<_> = transcript_qa
        .entries
        .iter()
        .filter(|e| e.kind == "tool")
        .collect();
    assert_eq!(qa_tools.len(), 2);
    assert_eq!(
        qa_tools[0].tool_name.as_deref(),
        Some("request_user_input_async")
    );
    assert_eq!(qa_tools[0].status.as_deref(), Some("completed"));
    assert_eq!(qa_tools[1].tool_name.as_deref(), Some("sleep"));
    assert_eq!(qa_tools[1].status.as_deref(), Some("completed"));
}

#[test]
fn resolver_resolves_rollout_by_exact_uuid_and_directory_structure() {
    let temp = Temp::new();
    let rollout_dir = temp.0.join("2026/10/13");
    fs::create_dir_all(&rollout_dir).unwrap();
    let rollout_file = rollout_dir.join(format!("rollout-2026-10-13T00-00-00-{THREAD_ID}.jsonl"));
    fs::write(&rollout_file, ROLLOUT_FIXTURE).unwrap();

    let resolver = CodexResolver::new(&temp.0).unwrap();
    let pane = make_pane(
        Some(THREAD_ID),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/fixture/work",
    );
    let (resolved_id, file) = resolver.resolve(THREAD_ID, &pane).unwrap();
    assert_eq!(resolved_id, THREAD_ID);

    let mut follower = Follower::for_agent(AgentType::Codex);
    assert!(follower.refresh_file(file, &resolved_id).unwrap());
    assert_eq!(
        follower
            .transcript
            .entries
            .iter()
            .filter(|e| e.kind == "message")
            .count(),
        4
    );
}

#[test]
fn resolver_fails_on_wrong_source_or_missing_reference() {
    let temp = Temp::new();
    let resolver = CodexResolver::new(&temp.0).unwrap();
    let adapter = CodexAdapter::new(Some(resolver));

    // Missing agent_session
    let mut pane = make_pane(None, "", "codex", "", "", "/fixture/work");
    pane.agent_session = None;
    assert_eq!(
        adapter.resolve_and_open(&pane).unwrap_err(),
        "session_reference_unavailable"
    );

    // Wrong source
    let pane_wrong_source = make_pane(
        Some(THREAD_ID),
        "herdr:claude",
        "codex",
        "codex",
        "id",
        "/fixture/work",
    );
    assert_eq!(
        adapter.resolve_and_open(&pane_wrong_source).unwrap_err(),
        "unsupported_session_source"
    );

    // Wrong agent in session
    let pane_wrong_agent = make_pane(
        Some(THREAD_ID),
        "herdr:codex",
        "codex",
        "claude",
        "id",
        "/fixture/work",
    );
    assert_eq!(
        adapter.resolve_and_open(&pane_wrong_agent).unwrap_err(),
        "unsupported_session_source"
    );

    // Wrong kind
    let pane_wrong_kind = make_pane(
        Some(THREAD_ID),
        "herdr:codex",
        "codex",
        "codex",
        "path",
        "/fixture/work",
    );
    assert_eq!(
        adapter.resolve_and_open(&pane_wrong_kind).unwrap_err(),
        "invalid_session_reference"
    );
}

#[test]
fn resolver_validates_thread_id_strictly_before_touching_filesystem() {
    let temp = Temp::new();
    let resolver = CodexResolver::new(&temp.0).unwrap();
    let adapter = CodexAdapter::new(Some(resolver));

    for bad_id in [
        "../etc/passwd",
        "../../f7b6ed8d-54fd-5ac3-bf0b-25eaa0e19912",
        "not-a-uuid",
        "",
        "f7b6ed8d-54fd-5ac3-bf0b-25eaa0e19912/extra",
        "f7b6ed8d-54fd-5ac3-bf0b-25eaa0e19912\0evil",
    ] {
        let pane = make_pane(
            Some(bad_id),
            "herdr:codex",
            "codex",
            "codex",
            "id",
            "/fixture/work",
        );
        assert_eq!(
            adapter.resolve_and_open(&pane).unwrap_err(),
            "invalid_session_reference",
            "Failed on bad id: {bad_id}"
        );
    }
}

#[test]
fn resolver_fails_on_thread_id_mismatch_between_filename_and_session_meta() {
    let temp = Temp::new();
    let rollout_dir = temp.0.join("2026/10/13");
    fs::create_dir_all(&rollout_dir).unwrap();
    let rollout_file = rollout_dir.join(format!("rollout-2026-10-13T00-00-00-{THREAD_ID}.jsonl"));

    // Write session_meta with DIFFERENT ID:
    let foreign_id = "00000000-0000-0000-0000-000000000000";
    let bad_first_row = json!({
        "type": "session_meta",
        "payload": {
            "id": foreign_id,
            "cwd": "/fixture/work"
        }
    });
    fs::write(&rollout_file, format!("{bad_first_row}\n")).unwrap();

    let resolver = CodexResolver::new(&temp.0).unwrap();
    let pane = make_pane(
        Some(THREAD_ID),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/fixture/work",
    );
    assert_eq!(
        resolver.resolve(THREAD_ID, &pane).unwrap_err(),
        "session_mismatch"
    );
}

#[test]
fn resolver_fails_closed_on_cwd_mismatch() {
    let temp = Temp::new();
    let rollout_dir = temp.0.join("2026/10/13");
    fs::create_dir_all(&rollout_dir).unwrap();
    let rollout_file = rollout_dir.join(format!("rollout-2026-10-13T00-00-00-{THREAD_ID}.jsonl"));
    fs::write(&rollout_file, ROLLOUT_FIXTURE).unwrap();

    let resolver = CodexResolver::new(&temp.0).unwrap();

    // Mismatched foreground cwd
    let pane_mismatch = make_pane(
        Some(THREAD_ID),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/unrelated/path",
    );
    assert_eq!(
        resolver.resolve(THREAD_ID, &pane_mismatch).unwrap_err(),
        "session_cwd_mismatch"
    );

    // Matching foreground cwd (even with trailing slash)
    let pane_matching_slash = make_pane(
        Some(THREAD_ID),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/fixture/work/",
    );
    assert!(resolver.resolve(THREAD_ID, &pane_matching_slash).is_ok());

    // Fallback to pane.cwd when foreground_cwd is None
    let mut pane_fallback_cwd = make_pane(
        Some(THREAD_ID),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/fixture/work",
    );
    pane_fallback_cwd.foreground_cwd = None;
    assert!(resolver.resolve(THREAD_ID, &pane_fallback_cwd).is_ok());

    // Mismatch in fallback pane.cwd
    pane_fallback_cwd.cwd = Some("/wrong/path".to_string());
    assert_eq!(
        resolver.resolve(THREAD_ID, &pane_fallback_cwd).unwrap_err(),
        "session_cwd_mismatch"
    );
}

#[test]
fn resolver_rejects_symlink_escape() {
    let temp = Temp::new();
    let rollout_dir = temp.0.join("2026/10/13");
    fs::create_dir_all(&rollout_dir).unwrap();

    // Create a target file outside the Codex root
    let outside_temp = Temp::new();
    let outside_file = outside_temp.0.join("secret.jsonl");
    fs::write(&outside_file, ROLLOUT_FIXTURE).unwrap();

    // Create a symlink in the rollout directory pointing outside
    let symlink_path = rollout_dir.join(format!("rollout-2026-10-13T00-00-00-{THREAD_ID}.jsonl"));
    std::os::unix::fs::symlink(&outside_file, &symlink_path).unwrap();

    let resolver = CodexResolver::new(&temp.0).unwrap();
    let pane = make_pane(
        Some(THREAD_ID),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/fixture/work",
    );
    // Symlink inside the tree must be ignored during scanning or fail confined open
    assert_eq!(
        resolver.resolve(THREAD_ID, &pane).unwrap_err(),
        "transcript_unavailable"
    );
}

#[test]
fn follower_handles_appended_lines() {
    let temp = Temp::new();
    let file_path = temp.0.join("rollout.jsonl");

    // Write initial records: session_meta + first user message
    let initial = format!(
        "{}\n{}\n",
        ROLLOUT_FIXTURE.lines().next().unwrap(),
        ROLLOUT_FIXTURE.lines().nth(3).unwrap()
    );
    fs::write(&file_path, &initial).unwrap();

    let mut follower = Follower::for_agent(AgentType::Codex);
    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());
    assert_eq!(follower.transcript.entries.len(), 1);
    assert_eq!(follower.transcript.entries[0].role.as_deref(), Some("user"));

    // Append tool call
    let tool_call = ROLLOUT_FIXTURE.lines().nth(7).unwrap();
    let mut append_file = fs::OpenOptions::new()
        .append(true)
        .open(&file_path)
        .unwrap();
    writeln!(append_file, "{tool_call}").unwrap();

    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());
    assert_eq!(follower.transcript.entries.len(), 2);
    assert_eq!(follower.transcript.entries[1].kind, "tool");
    assert_eq!(
        follower.transcript.entries[1].status.as_deref(),
        Some("running")
    );

    // Append tool output
    let tool_output = ROLLOUT_FIXTURE.lines().nth(8).unwrap();
    writeln!(append_file, "{tool_output}").unwrap();

    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());
    assert_eq!(follower.transcript.entries.len(), 2);
    assert_eq!(
        follower.transcript.entries[1].status.as_deref(),
        Some("completed")
    );
}

#[test]
fn follower_handles_oversized_lines() {
    let temp = Temp::new();
    let file_path = temp.0.join("rollout.jsonl");

    // 1. Oversized user message
    let big_text = "a".repeat(MAX_RECORD + 2048);
    let meta_line = json!({
        "type": "session_meta",
        "payload": {
            "id": THREAD_ID,
            "cwd": "/fixture/work"
        }
    })
    .to_string();
    let big_message_line = json!({
        "type": "response_item",
        "payload": {
            "type": "message",
            "id": "big-msg-1",
            "role": "user",
            "content": [
                {
                    "type": "input_text",
                    "text": big_text
                }
            ]
        }
    })
    .to_string();

    // 2. Oversized tool call
    let big_args = "x".repeat(MAX_RECORD + 1024);
    let big_tool_line = json!({
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "id": "big-tool-1",
            "call_id": "call-1",
            "name": "large_function",
            "arguments": big_args
        }
    })
    .to_string();

    fs::write(
        &file_path,
        format!("{meta_line}\n{big_message_line}\n{big_tool_line}\n"),
    )
    .unwrap();

    let mut follower = Follower::for_agent(AgentType::Codex);
    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());

    let transcript = &follower.transcript;
    assert_eq!(transcript.entries.len(), 2);
    // Bounded text limit is 8192
    assert!(transcript.entries[0].text.len() <= 8192);
    assert!(transcript.entries[0].truncated);
    assert_eq!(transcript.entries[1].kind, "tool");
    assert!(transcript.entries[1].details.as_ref().unwrap().len() <= 8192);
    assert!(transcript.entries[1].truncated);
}

#[test]
fn follower_record_content_preserves_content_references() {
    let temp = Temp::new();
    let file_path = temp.0.join("rollout.jsonl");
    fs::write(&file_path, ROLLOUT_FIXTURE).unwrap();

    let mut follower = Follower::with_content_for_agent(AgentType::Codex);
    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());

    let transcript = &follower.transcript;
    assert!(!transcript.entries.is_empty());

    for entry in &transcript.entries {
        if entry.kind == "message" {
            assert!(!entry.record_refs.is_empty());
            for record_ref in &entry.record_refs {
                let (r_bytes, r_total) = follower.read_content(record_ref, 0).unwrap();
                assert!(!r_bytes.is_empty());
                assert_eq!(r_bytes.len(), r_total);
            }
        }
    }
}
