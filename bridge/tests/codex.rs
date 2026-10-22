use herdcats_bridge::{
    adapter::Adapter,
    claude::{MAX_ENTRY_ID, Transcript},
    codex::{CodexAdapter, CodexResolver},
    follower::{AgentType, Follower, MAX_RECORD, MAX_SOURCE},
    herdr::{Pane, SessionRef},
};
use serde_json::json;
use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
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
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0].role.as_deref(), Some("user"));
    assert_eq!(messages[1].role.as_deref(), Some("user"));
    assert!(
        messages[1]
            .text
            .contains("Run exactly this shell command: echo herdcats-probe > probe2.txt")
    );
    assert_eq!(messages[2].role.as_deref(), Some("assistant"));
    assert_eq!(messages[3].role.as_deref(), Some("assistant"));
    assert_eq!(messages[3].text, "Turn interrupted");
    assert_eq!(messages[3].status.as_deref(), Some("failed"));
    assert_eq!(messages[4].role.as_deref(), Some("user"));

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
        5
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

#[test]
fn resolver_v7_timestamp_maps_to_day_dirs_and_resolves() {
    let temp = Temp::new();
    let thread_v7 = "01a14495-5600-7000-8000-000000000001";
    let ts_ms = herdcats_bridge::codex::parse_uuid_v7_timestamp_ms(thread_v7).unwrap();
    assert_eq!(ts_ms, 1792152000000); // 2026-10-16 12:00:00 UTC
    let candidate_dirs = herdcats_bridge::codex::v7_candidate_day_dirs(ts_ms);
    assert_eq!(
        candidate_dirs,
        vec![(2026, 10, 15), (2026, 10, 16), (2026, 10, 17)]
    );

    let day_dir = temp.0.join("2026/10/16");
    fs::create_dir_all(&day_dir).unwrap();
    let rollout_file = day_dir.join(format!("rollout-2026-10-16T12-00-00-{thread_v7}.jsonl"));
    fs::write(
        &rollout_file,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread_v7, "cwd": "/test/cwd"}})
        ),
    )
    .unwrap();

    let resolver = CodexResolver::new(&temp.0).unwrap();
    let pane = make_pane(
        Some(thread_v7),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/test/cwd",
    );
    let (resolved_id, file) = resolver.resolve(thread_v7, &pane).unwrap();
    assert_eq!(resolved_id, thread_v7);
    assert_eq!(
        file.metadata().unwrap().len(),
        rollout_file.metadata().unwrap().len()
    );
}

#[test]
fn resolver_resolves_file_in_neighbouring_day_dir() {
    let temp = Temp::new();
    let thread_v7 = "01a14495-5600-7000-8000-000000000001"; // 2026-10-16 UTC

    // Place file in neighbouring day 2026/10/15 (e.g. UTC-10 timezone offset)
    let day_dir = temp.0.join("2026/10/15");
    fs::create_dir_all(&day_dir).unwrap();
    let rollout_file = day_dir.join(format!("rollout-2026-10-15T22-00-00-{thread_v7}.jsonl"));
    fs::write(
        &rollout_file,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread_v7, "cwd": "/test/cwd"}})
        ),
    )
    .unwrap();

    let resolver = CodexResolver::new(&temp.0).unwrap();
    let pane = make_pane(
        Some(thread_v7),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/test/cwd",
    );
    let (resolved_id, _) = resolver.resolve(thread_v7, &pane).unwrap();
    assert_eq!(resolved_id, thread_v7);

    // If file was in 2026/10/10 (outside +/- 1 day window), resolution must fail
    let outside_dir = temp.0.join("2026/10/10");
    fs::create_dir_all(&outside_dir).unwrap();
    let thread_v7_outside = "01a14495-5600-7000-8000-000000000002";
    let rollout_outside = outside_dir.join(format!(
        "rollout-2026-10-10T12-00-00-{thread_v7_outside}.jsonl"
    ));
    fs::write(
        &rollout_outside,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread_v7_outside, "cwd": "/test/cwd"}})
        ),
    )
    .unwrap();
    let pane_outside = make_pane(
        Some(thread_v7_outside),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/test/cwd",
    );
    assert_eq!(
        resolver
            .resolve(thread_v7_outside, &pane_outside)
            .unwrap_err(),
        "transcript_unavailable"
    );
}

#[test]
fn adapter_cache_hit_still_reverifies_session_meta_for_v7() {
    let temp = Temp::new();
    let thread_v7 = "01a14495-5600-7000-8000-000000000001";
    let day_dir = temp.0.join("2026/10/16");
    fs::create_dir_all(&day_dir).unwrap();
    let rollout_file = day_dir.join(format!("rollout-2026-10-16T12-00-00-{thread_v7}.jsonl"));
    fs::write(
        &rollout_file,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread_v7, "cwd": "/test/cwd"}})
        ),
    )
    .unwrap();

    let resolver = CodexResolver::new(&temp.0).unwrap();
    let meta_read_counter = resolver.meta_read_count.clone();
    let adapter = CodexAdapter::new(Some(resolver));
    let pane = make_pane(
        Some(thread_v7),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/test/cwd",
    );

    // First call: cold cache, reads session_meta
    let (id1, _) = adapter.resolve_and_open(&pane).unwrap();
    assert_eq!(id1, thread_v7);
    assert_eq!(meta_read_counter.load(Ordering::SeqCst), 1);

    // Second call: warm cache with an identical stamp still re-reads session_meta
    let (id2, _) = adapter.resolve_and_open(&pane).unwrap();
    assert_eq!(id2, thread_v7);
    assert_eq!(meta_read_counter.load(Ordering::SeqCst), 2);

    // Same-length in-place rewrite of the first record with the mtime restored
    let mtime = fs::metadata(&rollout_file).unwrap().modified().unwrap();
    let rewritten = fs::read_to_string(&rollout_file)
        .unwrap()
        .replace("/test/cwd", "/test/cwx");
    fs::write(&rollout_file, rewritten).unwrap();
    fs::File::options()
        .write(true)
        .open(&rollout_file)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    assert_eq!(
        adapter.resolve_and_open(&pane).unwrap_err(),
        "session_cwd_mismatch"
    );

    // Third call after removing the file: fails and clears cache
    fs::remove_file(&rollout_file).unwrap();
    assert_eq!(
        adapter.resolve_and_open(&pane).unwrap_err(),
        "transcript_unavailable"
    );
}

#[test]
fn adapter_non_v7_cache_skips_full_walk_within_30s() {
    let temp = Temp::new();
    let thread_non_v7 = "11111111-2222-4333-8444-555555555555";
    let day_dir = temp.0.join("2026/05/01");
    fs::create_dir_all(&day_dir).unwrap();
    let rollout_file = day_dir.join(format!("rollout-2026-05-01T10-00-00-{thread_non_v7}.jsonl"));
    fs::write(
        &rollout_file,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread_non_v7, "cwd": "/test/cwd"}})
        ),
    )
    .unwrap();

    let resolver = CodexResolver::new(&temp.0).unwrap();
    let scan_counter = resolver.scan_count.clone();
    let adapter = CodexAdapter::new(Some(resolver));
    let pane = make_pane(
        Some(thread_non_v7),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/test/cwd",
    );

    // First call: cold cache, must scan
    let (id1, _) = adapter.resolve_and_open(&pane).unwrap();
    assert_eq!(id1, thread_non_v7);
    assert_eq!(scan_counter.load(Ordering::SeqCst), 1);

    // Second call: within 30s, warm cache hit, must NOT full-walk scan again
    let (id2, _) = adapter.resolve_and_open(&pane).unwrap();
    assert_eq!(id2, thread_non_v7);
    assert_eq!(
        scan_counter.load(Ordering::SeqCst),
        1,
        "Cache hit within 30s must not re-scan directory"
    );
}

#[test]
fn v7_duplicate_appears_after_first_lookup_fails_with_ambiguous_session() {
    let temp = Temp::new();
    let thread_v7 = "01a14495-5600-7000-8000-000000000001";
    let day_dir = temp.0.join("2026/10/16");
    fs::create_dir_all(&day_dir).unwrap();
    let rollout_file1 = day_dir.join(format!("rollout-2026-10-16T12-00-00-{thread_v7}.jsonl"));
    fs::write(
        &rollout_file1,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread_v7, "cwd": "/test/cwd"}})
        ),
    )
    .unwrap();

    let resolver = CodexResolver::new(&temp.0).unwrap();
    let adapter = CodexAdapter::new(Some(resolver));
    let pane = make_pane(
        Some(thread_v7),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/test/cwd",
    );

    // First lookup succeeds
    let (id1, _) = adapter.resolve_and_open(&pane).unwrap();
    assert_eq!(id1, thread_v7);

    // Duplicate appears in neighbouring day dir (within +-1 day)
    let day_dir_neighbour = temp.0.join("2026/10/15");
    fs::create_dir_all(&day_dir_neighbour).unwrap();
    let rollout_file2 =
        day_dir_neighbour.join(format!("rollout-2026-10-15T12-00-00-{thread_v7}.jsonl"));
    fs::write(
        &rollout_file2,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread_v7, "cwd": "/test/cwd"}})
        ),
    )
    .unwrap();

    // Next poll fails with ambiguous_session
    assert_eq!(
        adapter.resolve_and_open(&pane).unwrap_err(),
        "ambiguous_session"
    );
}

#[test]
fn non_v7_duplicate_detected_after_interval_with_injected_clock() {
    let temp = Temp::new();
    // Non-v7 thread ID (version 4)
    let thread_non_v7 = "11111111-2222-4333-8444-555555555555";
    let day_dir1 = temp.0.join("2026/05/01");
    fs::create_dir_all(&day_dir1).unwrap();
    let rollout_file1 = day_dir1.join(format!("rollout-2026-05-01T10-00-00-{thread_non_v7}.jsonl"));
    fs::write(
        &rollout_file1,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread_non_v7, "cwd": "/test/cwd"}})
        ),
    )
    .unwrap();

    let start = Instant::now();
    let offset_secs = Arc::new(AtomicU64::new(0));
    let offset_clone = offset_secs.clone();
    let clock = Arc::new(move || start + Duration::from_secs(offset_clone.load(Ordering::SeqCst)));

    let resolver = CodexResolver::new(&temp.0).unwrap();
    let adapter = CodexAdapter::with_clock(Some(resolver), clock);
    let pane = make_pane(
        Some(thread_non_v7),
        "herdr:codex",
        "codex",
        "codex",
        "id",
        "/test/cwd",
    );

    // First lookup at t=0s: full walk, succeeds
    let (id1, _) = adapter.resolve_and_open(&pane).unwrap();
    assert_eq!(id1, thread_non_v7);

    // Duplicate appears in another directory
    let day_dir2 = temp.0.join("2026/06/01");
    fs::create_dir_all(&day_dir2).unwrap();
    let rollout_file2 = day_dir2.join(format!("rollout-2026-06-01T10-00-00-{thread_non_v7}.jsonl"));
    fs::write(
        &rollout_file2,
        format!(
            "{}\n",
            json!({"type": "session_meta", "payload": {"id": thread_non_v7, "cwd": "/test/cwd"}})
        ),
    )
    .unwrap();

    // At t=10s (< 30s): full walk is NOT redone, uses cached path
    offset_secs.store(10, Ordering::SeqCst);
    let (id2, _) = adapter.resolve_and_open(&pane).unwrap();
    assert_eq!(id2, thread_non_v7);

    // At t=31s (>= 30s): full walk is redone, detects duplicate, fails closed
    offset_secs.store(31, Ordering::SeqCst);
    assert_eq!(
        adapter.resolve_and_open(&pane).unwrap_err(),
        "ambiguous_session"
    );
}

#[test]
fn civil_from_days_february_boundaries() {
    use herdcats_bridge::codex::civil_from_days;

    // 2026-02-26 (regression for civil_from_days divisor formula)
    assert_eq!(civil_from_days(20510), (2026, 2, 26));

    // Leap year 2024
    assert_eq!(civil_from_days(19781), (2024, 2, 28));
    assert_eq!(civil_from_days(19782), (2024, 2, 29));
    assert_eq!(civil_from_days(19783), (2024, 3, 1));

    // Non-leap year 2023
    assert_eq!(civil_from_days(19416), (2023, 2, 28));
    assert_eq!(civil_from_days(19417), (2023, 3, 1));

    // Century leap year 2000
    assert_eq!(civil_from_days(11016), (2000, 2, 29));

    // Century non-leap year 1900
    assert_eq!(civil_from_days(-25509), (1900, 2, 28));
    assert_eq!(civil_from_days(-25508), (1900, 3, 1));
}

#[test]
fn preserve_codex_record_unfamiliar_record_creates_bounded_bubble() {
    let mut transcript = Transcript::default();

    // 1. Unfamiliar top-level record
    let unknown_row = json!({
        "type": "future_experiment",
        "payload": {"param": 42}
    });
    let bytes1 = serde_json::to_vec(&unknown_row).unwrap();
    transcript
        .preserve_codex_record(&bytes1, THREAD_ID, 100)
        .unwrap();

    assert_eq!(transcript.entries.len(), 1);
    assert_eq!(transcript.entries[0].kind, "record");
    assert_eq!(transcript.entries[0].text, "future_experiment");
    assert_eq!(
        transcript.entries[0].content_ref.as_deref(),
        Some(format!("source:100:{}", bytes1.len()).as_str())
    );

    // 2. Unfamiliar response_item
    let unknown_item = json!({
        "type": "response_item",
        "payload": {
            "type": "future_widget",
            "content": "custom visualization"
        }
    });
    let bytes2 = serde_json::to_vec(&unknown_item).unwrap();
    transcript
        .preserve_codex_record(&bytes2, THREAD_ID, 300)
        .unwrap();

    assert_eq!(transcript.entries.len(), 2);
    assert_eq!(transcript.entries[1].kind, "record");
    assert_eq!(transcript.entries[1].text, "future_widget");
    assert_eq!(
        transcript.entries[1].content_ref.as_deref(),
        Some(format!("source:300:{}", bytes2.len()).as_str())
    );
}

#[test]
fn synthetic_rollout_skips_routine_records_and_preserves_only_agent_message_and_unknown() {
    let temp = Temp::new();
    let file_path = temp.0.join("rollout.jsonl");

    let records = vec![
        json!({"type": "session_meta", "payload": {"id": THREAD_ID, "cwd": "/test/cwd"}}),
        json!({"type": "response_item", "payload": {"type": "reasoning", "summary": [], "encrypted_content": "enc-bytes-123"}}),
        json!({"type": "response_item", "payload": {"type": "compaction", "encrypted_content": "comp-bytes-456"}}),
        json!({"type": "compacted", "payload": {"index": 1}}),
        json!({"type": "event_msg", "payload": {"type": "thread_settings_applied"}}),
        json!({"type": "inter_agent_communication_metadata", "payload": {"trigger_turn": true}}),
        json!({"type": "turn_context", "payload": {"model": "o3"}}),
        json!({"type": "environment_context", "payload": {"cwd": "/test/cwd"}}),
        json!({"type": "event_msg", "payload": {"type": "task_started"}}),
        json!({"type": "event_msg", "payload": {"type": "token_count"}}),
        json!({"type": "event_msg", "payload": {"type": "token_usage"}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed"}}),
        json!({"type": "event_msg", "payload": {"type": "task_complete"}}),
        json!({"type": "event_msg", "payload": {"type": "turn_aborted", "turn_id": "turn-test-1"}}),
        json!({
            "type": "response_item",
            "payload": {
                "type": "agent_message",
                "author": "planner",
                "recipient": "worker",
                "content": [{"type": "text", "text": "synthetic message"}]
            }
        }),
        json!({
            "type": "genuinely_unknown_record",
            "payload": {"arbitrary_field": 42}
        }),
    ];

    let mut content = String::new();
    for r in &records {
        content.push_str(&r.to_string());
        content.push('\n');
    }
    fs::write(&file_path, content).unwrap();

    let mut follower = Follower::with_content_for_agent(AgentType::Codex);
    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());

    let transcript = &follower.transcript;

    // Routine records must produce ZERO record entries except agent_message and genuinely_unknown_record
    let record_entries: Vec<_> = transcript
        .entries
        .iter()
        .filter(|e| e.kind == "record")
        .collect();

    assert_eq!(
        record_entries.len(),
        2,
        "Only agent_message and unknown type should produce record entries, got: {:?}",
        record_entries
            .iter()
            .map(|e| (&e.text, &e.kind))
            .collect::<Vec<_>>()
    );

    assert_eq!(record_entries[0].text, "agent_message");
    assert_eq!(record_entries[1].text, "genuinely_unknown_record");

    // turn_aborted should render a short status entry ("Turn interrupted"), not a record dump
    let turn_aborted_entry = transcript
        .entries
        .iter()
        .find(|e| e.id == "turn_aborted:turn-test-1");
    assert!(turn_aborted_entry.is_some());
    let entry = turn_aborted_entry.unwrap();
    assert_eq!(entry.kind, "message");
    assert_eq!(entry.text, "Turn interrupted");
    assert_eq!(entry.status.as_deref(), Some("failed"));
    assert_eq!(entry.details, None);
}

#[test]
fn oversized_skip_types_produce_no_entries_and_turn_aborted_keeps_turn_id() {
    let temp = Temp::new();
    let file_path = temp.0.join("rollout.jsonl");

    let pad = "x".repeat(MAX_RECORD + 1024);
    let lines = vec![
        json!({
            "type": "session_meta",
            "payload": { "id": THREAD_ID, "cwd": "/fixture/work" }
        })
        .to_string(),
        json!({
            "type": "response_item",
            "payload": { "type": "compaction", "encrypted_content": pad }
        })
        .to_string(),
        json!({
            "type": "response_item",
            "payload": { "type": "reasoning", "summary": [], "encrypted_content": pad }
        })
        .to_string(),
        json!({
            "type": "response_item",
            "payload": { "type": "message", "role": "developer", "content": pad }
        })
        .to_string(),
        json!({
            "type": "response_item",
            "payload": { "type": "message", "role": "system", "content": pad }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": { "type": "thread_settings_applied", "data": pad }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": { "type": "task_started", "data": pad }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": { "type": "item_completed", "data": pad }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": { "type": "task_complete", "data": pad }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": { "type": "token_count", "data": pad }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": { "type": "token_usage", "data": pad }
        })
        .to_string(),
        json!({
            "type": "turn_context",
            "payload": { "data": pad }
        })
        .to_string(),
        json!({
            "type": "environment_context",
            "payload": { "data": pad }
        })
        .to_string(),
        json!({
            "type": "world_state",
            "payload": { "data": pad }
        })
        .to_string(),
        json!({
            "type": "token_usage_record",
            "payload": { "data": pad }
        })
        .to_string(),
        json!({
            "type": "compacted",
            "payload": { "data": pad }
        })
        .to_string(),
        json!({
            "type": "inter_agent_communication_metadata",
            "payload": { "data": pad }
        })
        .to_string(),
        json!({
            "type": "event_msg",
            "payload": {
                "type": "turn_aborted",
                "turn_id": "turn-oversized-42",
                "data": pad
            }
        })
        .to_string(),
    ];

    fs::write(&file_path, lines.join("\n") + "\n").unwrap();

    let mut follower = Follower::with_content_for_agent(AgentType::Codex);
    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());

    let transcript = &follower.transcript;

    // All 16 skip record types must produce zero entries.
    // The only emitted entry must be the turn_aborted entry with turn_id in its id.
    assert_eq!(
        transcript.entries.len(),
        1,
        "Expected exactly 1 entry (turn_aborted), got {:?}",
        transcript.entries.iter().map(|e| &e.id).collect::<Vec<_>>()
    );

    let entry = &transcript.entries[0];
    assert_eq!(entry.id, "turn_aborted:turn-oversized-42");
    assert_eq!(entry.kind, "message");
    assert_eq!(entry.text, "Turn interrupted");
    assert_eq!(entry.status.as_deref(), Some("failed"));
    assert_eq!(transcript.unknown_records, 0);
}

#[test]
fn preserve_codex_record_image_only_message_creates_bounded_bubble() {
    let mut transcript = Transcript::default();

    let image_msg = json!({
        "type": "response_item",
        "payload": {
            "type": "message",
            "id": "msg_img_123",
            "role": "user",
            "content": [
                {
                    "type": "image_url",
                    "image_url": {
                        "url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
                    }
                }
            ]
        }
    });

    // 1. apply_codex inserts nothing because text is empty
    transcript.apply_codex(&image_msg, THREAD_ID, 0).unwrap();
    assert_eq!(transcript.entries.len(), 0);

    // 2. preserve_codex_record creates a bounded record bubble because attach_record finds no entry
    let bytes = serde_json::to_vec(&image_msg).unwrap();
    transcript
        .preserve_codex_record(&bytes, THREAD_ID, 500)
        .unwrap();

    assert_eq!(transcript.entries.len(), 1);
    assert_eq!(transcript.entries[0].kind, "record");
    assert_eq!(transcript.entries[0].text, "message");
    assert_eq!(
        transcript.entries[0].content_ref.as_deref(),
        Some(format!("source:500:{}", bytes.len()).as_str())
    );
    assert!(transcript.entries[0].details.is_some());

    // 3. Re-preserving the same offset does not duplicate the record entry
    transcript
        .preserve_codex_record(&bytes, THREAD_ID, 500)
        .unwrap();
    assert_eq!(transcript.entries.len(), 1);
}

#[test]
fn follower_preserves_image_only_user_message_in_rollout() {
    let temp = Temp::new();
    let file_path = temp.0.join("image_rollout.jsonl");

    let records = vec![
        json!({"type": "session_meta", "payload": {"id": THREAD_ID, "cwd": "/test/cwd"}}),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg_user_img",
                "role": "user",
                "content": [
                    {
                        "type": "image_url",
                        "image_url": {
                            "url": "data:image/jpeg;base64,/9j/4AAQSkZJRg=="
                        }
                    }
                ]
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg_assistant_reply",
                "role": "assistant",
                "content": [
                    {
                        "type": "text",
                        "text": "I see the image."
                    }
                ]
            }
        }),
    ];

    let mut content = String::new();
    for r in &records {
        content.push_str(&r.to_string());
        content.push('\n');
    }
    fs::write(&file_path, content).unwrap();

    let mut follower = Follower::with_content_for_agent(AgentType::Codex);
    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());

    let transcript = &follower.transcript;

    // Both the image-only message (as a record bubble) and the assistant text message must exist
    assert_eq!(
        transcript.entries.len(),
        2,
        "Expected 2 entries: 1 record bubble for image input and 1 assistant message, got: {:?}",
        transcript
            .entries
            .iter()
            .map(|e| (&e.kind, &e.text))
            .collect::<Vec<_>>()
    );

    let img_entry = &transcript.entries[0];
    assert_eq!(img_entry.kind, "record");
    assert_eq!(img_entry.text, "message");
    assert!(img_entry.content_ref.is_some());

    let text_entry = &transcript.entries[1];
    assert_eq!(text_entry.kind, "message");
    assert_eq!(text_entry.text, "I see the image.");
    assert!(!text_entry.record_refs.is_empty());
}

#[test]
fn oversized_call_id_turn_id_and_error_id_rejected_and_page_cursor_works() {
    let mut transcript = Transcript::default();
    let oversized = "x".repeat(300);

    // 1. function_call: oversized call_id fails closed with invalid_transcript
    let bad_fc = json!({
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "id": "fc-valid-id-1",
            "call_id": oversized,
            "name": "exec_cmd",
            "arguments": "{}"
        }
    });
    assert_eq!(
        transcript.apply_codex(&bad_fc, THREAD_ID, 0),
        Err("invalid_transcript")
    );

    // Valid function_call is accepted
    let valid_fc = json!({
        "type": "response_item",
        "payload": {
            "type": "function_call",
            "id": "fc-valid-id-1",
            "call_id": "fc-call-id-1",
            "name": "exec_cmd",
            "arguments": "{}"
        }
    });
    transcript.apply_codex(&valid_fc, THREAD_ID, 0).unwrap();
    assert_eq!(transcript.entries.len(), 1);

    // 2. custom_tool_call: oversized call_id fails closed with invalid_transcript
    let bad_ctc = json!({
        "type": "response_item",
        "payload": {
            "type": "custom_tool_call",
            "id": "ctc-valid-id-2",
            "call_id": oversized,
            "name": "custom_exec",
            "input": "{}"
        }
    });
    assert_eq!(
        transcript.apply_codex(&bad_ctc, THREAD_ID, 0),
        Err("invalid_transcript")
    );

    // Valid custom_tool_call (fallback to id when call_id is absent) is accepted
    let valid_ctc = json!({
        "type": "response_item",
        "payload": {
            "type": "custom_tool_call",
            "id": "ctc-valid-id-2",
            "name": "custom_exec",
            "input": "{}"
        }
    });
    transcript.apply_codex(&valid_ctc, THREAD_ID, 0).unwrap();
    assert_eq!(transcript.entries.len(), 2);

    // 3. event_msg turn_aborted: oversized turn_id fails closed with invalid_transcript
    let bad_turn = json!({
        "type": "event_msg",
        "payload": {
            "type": "turn_aborted",
            "turn_id": oversized
        }
    });
    assert_eq!(
        transcript.apply_codex(&bad_turn, THREAD_ID, 0),
        Err("invalid_transcript")
    );

    // event_msg turn_aborted: oversized root_turn_id (when turn_id absent) fails closed
    let bad_root_turn = json!({
        "type": "event_msg",
        "payload": {
            "type": "turn_aborted",
            "root_turn_id": oversized
        }
    });
    assert_eq!(
        transcript.apply_codex(&bad_root_turn, THREAD_ID, 0),
        Err("invalid_transcript")
    );

    // Valid turn_aborted is accepted
    let valid_turn = json!({
        "type": "event_msg",
        "payload": {
            "type": "turn_aborted",
            "turn_id": "turn-valid-3"
        }
    });
    transcript.apply_codex(&valid_turn, THREAD_ID, 0).unwrap();
    assert_eq!(transcript.entries.len(), 3);

    // 4. response_item error: oversized id fails closed with invalid_transcript
    let bad_err = json!({
        "type": "response_item",
        "payload": {
            "type": "error",
            "id": oversized,
            "message": "Error details"
        }
    });
    assert_eq!(
        transcript.apply_codex(&bad_err, THREAD_ID, 0),
        Err("invalid_transcript")
    );

    // Valid error without id uses offset fallback
    let valid_err = json!({
        "type": "response_item",
        "payload": {
            "type": "error",
            "message": "Recoverable failure"
        }
    });
    transcript.apply_codex(&valid_err, THREAD_ID, 0).unwrap();
    assert_eq!(transcript.entries.len(), 4);

    // 5. Assert page and cursor still work on the resulting transcript entries
    let page_result = herdcats_bridge::service::page(&transcript.entries, transcript.entries.len());
    let page_entries = page_result["entries"].as_array().unwrap();
    assert_eq!(page_entries.len(), 4);
    assert_eq!(page_result["has_more"], false);
    assert_eq!(page_result["before"], "tool:fc-call-id-1");

    // All entry keys remain safely bounded
    for entry in &transcript.entries {
        assert!(
            entry.id.len() <= MAX_ENTRY_ID,
            "Entry id too long: {}",
            entry.id
        );
    }
}

#[test]
fn follower_fails_closed_on_oversized_codex_entry_keys() {
    let temp = Temp::new();
    let file_path = temp.0.join("bad_keys_rollout.jsonl");

    let records = vec![
        json!({"type": "session_meta", "payload": {"id": THREAD_ID, "cwd": "/test/cwd"}}),
        json!({
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "id": "valid-fc-id",
                "call_id": "x".repeat(300),
                "name": "test_tool",
                "arguments": "{}"
            }
        }),
    ];

    let mut content = String::new();
    for r in &records {
        content.push_str(&r.to_string());
        content.push('\n');
    }
    fs::write(&file_path, content).unwrap();

    let mut follower = Follower::with_content_for_agent(AgentType::Codex);
    assert_eq!(
        follower.refresh(&file_path, THREAD_ID).unwrap_err(),
        "invalid_transcript"
    );
}

#[test]
fn resolver_fails_closed_when_first_line_exceeds_max_record_without_newline() {
    let temp = Temp::new();
    let rollout_dir = temp.0.join("2026/10/13");
    fs::create_dir_all(&rollout_dir).unwrap();
    let rollout_file = rollout_dir.join(format!("rollout-2026-10-13T00-00-00-{THREAD_ID}.jsonl"));

    // Write a first line larger than MAX_RECORD (1 MiB) with no newline
    let large_unterminated = vec![b'a'; MAX_RECORD + 4096];
    fs::write(&rollout_file, large_unterminated).unwrap();

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
        "source_too_large"
    );
}

#[test]
fn resolver_fails_closed_when_file_exceeds_max_source() {
    let temp = Temp::new();
    let rollout_dir = temp.0.join("2026/10/13");
    fs::create_dir_all(&rollout_dir).unwrap();
    let rollout_file = rollout_dir.join(format!("rollout-2026-10-13T00-00-00-{THREAD_ID}.jsonl"));

    // Create sparse file larger than MAX_SOURCE (64 MiB)
    let file = fs::File::create(&rollout_file).unwrap();
    file.set_len(MAX_SOURCE + 1024).unwrap();

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
        "source_too_large"
    );
}

#[test]
fn resolver_rejects_relative_root() {
    assert_eq!(
        CodexResolver::new(std::path::Path::new("relative/sessions")).err(),
        Some("transcript_root_unavailable")
    );
}

#[test]
fn turn_id_bounds_to_client_entry_id_limit() {
    // 1. A 251-byte turn_id produces an entry ID of exactly 264 bytes (13 + 251 = 264), which passes
    let id_251 = "t".repeat(251);
    let mut transcript = Transcript::default();
    let record_251 = json!({
        "type": "event_msg",
        "payload": {
            "type": "turn_aborted",
            "turn_id": id_251
        }
    });
    assert!(transcript.apply_codex(&record_251, THREAD_ID, 0).is_ok());
    assert_eq!(transcript.entries.len(), 1);
    let entry = &transcript.entries[0];
    assert_eq!(entry.id.len(), 264);
    assert_eq!(entry.id.len(), MAX_ENTRY_ID);
    assert_eq!(entry.id, format!("turn_aborted:{id_251}"));

    // 2. A turn_id of 252..=256 bytes fails closed with invalid_transcript
    for len in 252..=256 {
        let bad_id = "t".repeat(len);
        let mut transcript = Transcript::default();
        let record = json!({
            "type": "event_msg",
            "payload": {
                "type": "turn_aborted",
                "turn_id": bad_id
            }
        });
        assert_eq!(
            transcript.apply_codex(&record, THREAD_ID, 0),
            Err("invalid_transcript"),
            "Expected turn_id of len {len} (total ID len {}) to fail closed",
            13 + len
        );
    }

    // 3. Same check for root_turn_id fallback (when turn_id is absent)
    let mut transcript = Transcript::default();
    let root_record_251 = json!({
        "type": "event_msg",
        "payload": {
            "type": "turn_aborted",
            "root_turn_id": id_251
        }
    });
    assert!(
        transcript
            .apply_codex(&root_record_251, THREAD_ID, 0)
            .is_ok()
    );
    assert_eq!(transcript.entries.len(), 1);
    assert_eq!(transcript.entries[0].id.len(), 264);

    for len in 252..=256 {
        let bad_id = "r".repeat(len);
        let mut transcript = Transcript::default();
        let record = json!({
            "type": "event_msg",
            "payload": {
                "type": "turn_aborted",
                "root_turn_id": bad_id
            }
        });
        assert_eq!(
            transcript.apply_codex(&record, THREAD_ID, 0),
            Err("invalid_transcript"),
            "Expected root_turn_id of len {len} (total ID len {}) to fail closed",
            13 + len
        );
    }

    // 4. Follower integration: 251-byte turn_id in rollout file passes, 252-byte fails closed
    let temp = Temp::new();
    let pass_file = temp.0.join("pass_rollout.jsonl");
    let pass_records = format!(
        "{}\n{}\n",
        json!({"type": "session_meta", "payload": {"id": THREAD_ID, "cwd": "/test/cwd"}}),
        json!({"type": "event_msg", "payload": {"type": "turn_aborted", "turn_id": "a".repeat(251)}})
    );
    fs::write(&pass_file, pass_records).unwrap();
    let mut follower = Follower::with_content_for_agent(AgentType::Codex);
    assert!(follower.refresh(&pass_file, THREAD_ID).unwrap());
    assert_eq!(follower.transcript.entries.len(), 1);
    assert_eq!(follower.transcript.entries[0].id.len(), 264);

    let fail_file = temp.0.join("fail_rollout.jsonl");
    let fail_records = format!(
        "{}\n{}\n",
        json!({"type": "session_meta", "payload": {"id": THREAD_ID, "cwd": "/test/cwd"}}),
        json!({"type": "event_msg", "payload": {"type": "turn_aborted", "turn_id": "a".repeat(252)}})
    );
    fs::write(&fail_file, fail_records).unwrap();
    let mut follower_fail = Follower::with_content_for_agent(AgentType::Codex);
    assert_eq!(
        follower_fail.refresh(&fail_file, THREAD_ID).unwrap_err(),
        "invalid_transcript"
    );
}

#[test]
fn all_codex_entry_ids_from_fixtures_and_synthetic_rollouts_within_limit() {
    let mut total_entries_checked = 0;

    let temp = Temp::new();

    // 1. Phase-1 fixtures
    let fixtures = [
        ("rollout", ROLLOUT_FIXTURE, THREAD_ID),
        ("pending", PENDING_FIXTURE, THREAD_ID),
        ("qa", QA_FIXTURE, "5f864d7a-0e47-526a-84c3-d03c78582686"),
    ];

    for (name, fixture_content, thread_id) in fixtures {
        let file_path = temp.0.join(format!("{name}.jsonl"));
        fs::write(&file_path, fixture_content).unwrap();

        // Check both with_content and standard follower
        for with_content in [false, true] {
            let mut follower = if with_content {
                Follower::with_content_for_agent(AgentType::Codex)
            } else {
                Follower::for_agent(AgentType::Codex)
            };

            assert!(
                follower.refresh(&file_path, thread_id).unwrap(),
                "Follower failed on fixture {name} (with_content: {with_content})"
            );

            assert!(
                !follower.transcript.entries.is_empty(),
                "Fixture {name} produced 0 entries"
            );

            for entry in &follower.transcript.entries {
                total_entries_checked += 1;
                assert!(
                    !entry.id.is_empty(),
                    "Entry id must not be empty in fixture {name}"
                );
                assert!(
                    entry.id.len() <= MAX_ENTRY_ID,
                    "Entry id '{}' exceeds MAX_ENTRY_ID ({} > {}) in fixture {name}",
                    entry.id,
                    entry.id.len(),
                    MAX_ENTRY_ID
                );
                assert!(
                    entry.id.len() <= 264,
                    "Entry id '{}' exceeds 264 wire bytes in fixture {name}",
                    entry.id
                );
            }
        }
    }

    // 2. Synthetic rollouts: routine record skips, agent message, unknown record, turn aborted, tool calls
    let synthetic_records = vec![
        json!({"type": "session_meta", "payload": {"id": THREAD_ID, "cwd": "/test/cwd"}}),
        json!({"type": "response_item", "payload": {"type": "reasoning", "summary": [], "encrypted_content": "enc-123"}}),
        json!({"type": "response_item", "payload": {"type": "compaction", "encrypted_content": "comp-456"}}),
        json!({"type": "compacted", "payload": {"index": 1}}),
        json!({"type": "event_msg", "payload": {"type": "thread_settings_applied"}}),
        json!({"type": "inter_agent_communication_metadata", "payload": {"trigger_turn": true}}),
        json!({"type": "turn_context", "payload": {"model": "o3"}}),
        json!({"type": "environment_context", "payload": {"cwd": "/test/cwd"}}),
        json!({"type": "event_msg", "payload": {"type": "task_started"}}),
        json!({"type": "event_msg", "payload": {"type": "token_count"}}),
        json!({"type": "event_msg", "payload": {"type": "token_usage"}}),
        json!({"type": "event_msg", "payload": {"type": "item_completed"}}),
        json!({"type": "event_msg", "payload": {"type": "task_complete"}}),
        json!({"type": "event_msg", "payload": {"type": "turn_aborted", "turn_id": "turn-test-1"}}),
        json!({
            "type": "response_item",
            "payload": {
                "type": "agent_message",
                "author": "planner",
                "recipient": "worker",
                "content": [{"type": "text", "text": "synthetic message"}]
            }
        }),
        json!({
            "type": "genuinely_unknown_record",
            "payload": {"arbitrary_field": 42}
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "id": "fc-synthetic-1",
                "call_id": "call-synthetic-1",
                "name": "synthetic_tool",
                "arguments": "{\"key\": \"value\"}"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "call-synthetic-1",
                "output": "synthetic tool output"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "custom_tool_call",
                "id": "ctc-synthetic-2",
                "name": "custom_tool",
                "input": "{\"arg\": 1}"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "error",
                "id": "err-synthetic-3",
                "message": "synthetic failure"
            }
        }),
    ];

    let synth_file = temp.0.join("synthetic.jsonl");
    let mut synth_content = String::new();
    for r in &synthetic_records {
        synth_content.push_str(&r.to_string());
        synth_content.push('\n');
    }
    fs::write(&synth_file, synth_content).unwrap();

    for with_content in [false, true] {
        let mut follower = if with_content {
            Follower::with_content_for_agent(AgentType::Codex)
        } else {
            Follower::for_agent(AgentType::Codex)
        };
        assert!(follower.refresh(&synth_file, THREAD_ID).unwrap());
        assert!(!follower.transcript.entries.is_empty());
        for entry in &follower.transcript.entries {
            total_entries_checked += 1;
            assert!(!entry.id.is_empty());
            assert!(entry.id.len() <= MAX_ENTRY_ID);
            assert!(entry.id.len() <= 264);
        }
    }

    assert!(
        total_entries_checked >= 40,
        "Expected at least 40 entries checked across fixtures and synthetic rollouts, got {total_entries_checked}"
    );
}

#[test]
fn rollout_with_id_less_errors_and_identified_error() {
    let temp = Temp::new();
    let file_path = temp.0.join("errors_rollout.jsonl");

    let records = vec![
        json!({"type": "session_meta", "payload": {"id": THREAD_ID, "cwd": "/test/cwd"}}),
        json!({
            "type": "response_item",
            "payload": {
                "type": "error",
                "message": "First ID-less error"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "error",
                "message": "Second ID-less error"
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "error",
                "id": "custom-err-id",
                "message": "Identified error"
            }
        }),
    ];

    let mut content = String::new();
    let mut offsets = Vec::new();
    let mut lengths = Vec::new();
    for r in &records {
        let line = format!("{}\n", r);
        offsets.push(content.len());
        lengths.push(line.len());
        content.push_str(&line);
    }
    fs::write(&file_path, &content).unwrap();

    let mut follower = Follower::with_content_for_agent(AgentType::Codex);
    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());

    let transcript = &follower.transcript;

    // 1. Two ID-less errors yield two distinct failed error entries with both messages,
    //    plus the identified error keeps its old ID.
    assert_eq!(transcript.entries.len(), 3);

    let err1 = &transcript.entries[0];
    let err2 = &transcript.entries[1];
    let err3 = &transcript.entries[2];

    assert_eq!(err1.id, format!("error:{}", offsets[1]));
    assert_eq!(err1.kind, "message");
    assert_eq!(err1.role.as_deref(), Some("assistant"));
    assert_eq!(err1.status.as_deref(), Some("failed"));
    assert_eq!(err1.text, "First ID-less error");
    assert_eq!(
        err1.record_refs,
        vec![format!("source:{}:{}", offsets[1], lengths[1])]
    );

    assert_eq!(err2.id, format!("error:{}", offsets[2]));
    assert_eq!(err2.kind, "message");
    assert_eq!(err2.role.as_deref(), Some("assistant"));
    assert_eq!(err2.status.as_deref(), Some("failed"));
    assert_eq!(err2.text, "Second ID-less error");
    assert_eq!(
        err2.record_refs,
        vec![format!("source:{}:{}", offsets[2], lengths[2])]
    );

    assert_eq!(err3.id, "message:custom-err-id");
    assert_eq!(err3.kind, "message");
    assert_eq!(err3.role.as_deref(), Some("assistant"));
    assert_eq!(err3.status.as_deref(), Some("failed"));
    assert_eq!(err3.text, "Identified error");
    assert_eq!(
        err3.record_refs,
        vec![format!("source:{}:{}", offsets[3], lengths[3])]
    );

    // No extra fallback record bubbles were created
    let record_entries: Vec<_> = transcript
        .entries
        .iter()
        .filter(|e| e.kind == "record")
        .collect();
    assert_eq!(record_entries.len(), 0);

    // 2. Both survive history paging / snapshot
    // Full snapshot page:
    let page_snapshot =
        herdcats_bridge::service::page(&transcript.entries, transcript.entries.len());
    let snapshot_entries = page_snapshot["entries"].as_array().unwrap();
    assert_eq!(snapshot_entries.len(), 3);
    assert_eq!(snapshot_entries[0]["text"], "First ID-less error");
    assert_eq!(snapshot_entries[1]["text"], "Second ID-less error");
    assert_eq!(snapshot_entries[2]["text"], "Identified error");
    assert_eq!(page_snapshot["before"], format!("error:{}", offsets[1]));

    // Page before third entry:
    let page_before_third = herdcats_bridge::service::page(&transcript.entries, 2);
    let before_third_entries = page_before_third["entries"].as_array().unwrap();
    assert_eq!(before_third_entries.len(), 2);
    assert_eq!(before_third_entries[0]["text"], "First ID-less error");
    assert_eq!(before_third_entries[1]["text"], "Second ID-less error");

    // Page before second entry:
    let page_before_second = herdcats_bridge::service::page(&transcript.entries, 1);
    let before_second_entries = page_before_second["entries"].as_array().unwrap();
    assert_eq!(before_second_entries.len(), 1);
    assert_eq!(before_second_entries[0]["text"], "First ID-less error");

    // All entry keys remain safely bounded <= MAX_ENTRY_ID
    for entry in &transcript.entries {
        assert!(entry.id.len() <= MAX_ENTRY_ID);
        assert!(entry.id.len() <= 264);
    }
}

#[test]
fn codex_injected_context_messages_are_hidden_while_real_prompts_are_shown() {
    let temp = Temp::new();
    let file_path = temp.0.join("rollout-context-filter.jsonl");

    let records = vec![
        json!({"type": "session_meta", "payload": {"id": THREAD_ID, "cwd": "/test/cwd"}}),
        // 1. Injected initial context with AGENTS.md + environment_context: must be HIDDEN
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-injected-agents-and-env",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "# AGENTS.md instructions\n\n<INSTRUCTIONS>\nBe helpful\n</INSTRUCTIONS>"
                    },
                    {
                        "type": "input_text",
                        "text": "<environment_context>\n  <cwd>/test/cwd</cwd>\n</environment_context>"
                    }
                ]
            }
        }),
        // 2. Injected bare environment_context: must be HIDDEN
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-injected-bare-env",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "  <environment_context>\n  <cwd>/test/cwd</cwd>\n</environment_context>"
                    }
                ]
            }
        }),
        // 3. Real user prompt mentioning "AGENTS.md" mid-text: must be SHOWN
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-real-midtext-mention",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "Please review AGENTS.md instructions for guidance on running tests."
                    }
                ]
            }
        }),
        // 4. Message mixing a real text part with a context part: must be SHOWN
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-real-mixed-context-and-prompt",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "# AGENTS.md instructions\n\nContext block"
                    },
                    {
                        "type": "input_text",
                        "text": "Here is my typed user question."
                    }
                ]
            }
        }),
        // 5. Additional injected cases: skill and turn_aborted
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-injected-skill",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "<skill>\n<name>grill-me</name>\n</skill>"
                    }
                ]
            }
        }),
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-injected-abort",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "<turn_aborted>\nThe user interrupted the turn."
                    }
                ]
            }
        }),
    ];

    let mut content = String::new();
    let mut offsets = Vec::new();
    let mut lengths = Vec::new();
    for r in &records {
        let line = format!("{}\n", r);
        offsets.push(content.len());
        lengths.push(line.len());
        content.push_str(&line);
    }
    fs::write(&file_path, &content).unwrap();

    let mut follower = Follower::with_content_for_agent(AgentType::Codex);
    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());

    let transcript = &follower.transcript;

    // Only the two real user messages should be present
    assert_eq!(transcript.entries.len(), 2);

    let first = &transcript.entries[0];
    assert_eq!(first.id, "message:msg-real-midtext-mention");
    assert_eq!(first.kind, "message");
    assert_eq!(first.role.as_deref(), Some("user"));
    assert_eq!(
        first.text,
        "Please review AGENTS.md instructions for guidance on running tests."
    );
    assert_eq!(
        first.record_refs,
        vec![format!("source:{}:{}", offsets[3], lengths[3])]
    );

    let second = &transcript.entries[1];
    assert_eq!(second.id, "message:msg-real-mixed-context-and-prompt");
    assert_eq!(second.kind, "message");
    assert_eq!(second.role.as_deref(), Some("user"));
    assert!(
        second
            .text
            .contains("Here is my typed user question.")
    );
    assert_eq!(
        second.record_refs,
        vec![format!("source:{}:{}", offsets[4], lengths[4])]
    );

    // No fallback record bubbles were created for skipped messages
    let record_bubbles: Vec<_> = transcript
        .entries
        .iter()
        .filter(|e| e.kind == "record")
        .collect();
    assert_eq!(record_bubbles.len(), 0);

    // Test history pagination and cursors
    let page_snapshot = herdcats_bridge::service::page(&transcript.entries, transcript.entries.len());
    let snapshot_entries = page_snapshot["entries"].as_array().unwrap();
    assert_eq!(snapshot_entries.len(), 2);
    assert_eq!(page_snapshot["before"], "message:msg-real-midtext-mention");

    let page_before_second = herdcats_bridge::service::page(&transcript.entries, 1);
    let before_second_entries = page_before_second["entries"].as_array().unwrap();
    assert_eq!(before_second_entries.len(), 1);
    assert_eq!(
        before_second_entries[0]["id"],
        "message:msg-real-midtext-mention"
    );
}

#[test]
fn is_codex_injected_context_edge_cases() {
    use herdcats_bridge::codex::{is_codex_injected_context, is_codex_injected_text};

    assert!(!is_codex_injected_text(""));
    assert!(!is_codex_injected_text("   "));
    assert!(!is_codex_injected_text("Hello AGENTS.md"));
    assert!(is_codex_injected_text("# AGENTS.md instructions"));
    assert!(is_codex_injected_text("# AGENTS.md instructions for /repo"));
    assert!(is_codex_injected_text("  \n# AGENTS.md instructions"));
    assert!(is_codex_injected_text("<environment_context>\n..."));
    assert!(is_codex_injected_text("<user_instructions>..."));
    assert!(is_codex_injected_text("<user_shell_command>..."));
    assert!(is_codex_injected_text("<turn_aborted>..."));
    assert!(is_codex_injected_text("<skill>\n<name>x</name>"));
    assert!(is_codex_injected_text("<skill name=\"x\">"));
    assert!(is_codex_injected_text("<subagent>\n..."));
    assert!(is_codex_injected_text("<subagent id=\"1\">"));
    assert!(is_codex_injected_text(
        "<subagent_notification>{\"agent\":\"researcher\",\"status\":\"completed\"}</subagent_notification>"
    ));
    assert!(!is_codex_injected_text("<subagent_notificationX"));
    assert!(!is_codex_injected_text("<send_user_message_question_reply>"));
    assert!(!is_codex_injected_text("<image name=[Image #1] path=\"/tmp/foo.png\">"));

    // Empty content
    assert!(!is_codex_injected_context(&json!("")));
    assert!(!is_codex_injected_context(&json!([])));
    assert!(!is_codex_injected_context(&json!(null)));

    // String content
    assert!(is_codex_injected_context(&json!("<environment_context>")));
    assert!(is_codex_injected_context(&json!(
        "<subagent_notification>{...}</subagent_notification>"
    )));
    assert!(!is_codex_injected_context(&json!(
        "<subagent_notificationX{...}</subagent_notificationX>"
    )));
    assert!(!is_codex_injected_context(&json!("Regular prompt")));

    // Non-text blocks (e.g. image) prevent message from being skipped as context
    let image_block = json!([{"type": "input_image", "url": "https://example.com/pic.png"}]);
    assert!(!is_codex_injected_context(&image_block));

    let mixed_image_and_context = json!([
        {"type": "input_image", "url": "https://example.com/pic.png"},
        {"type": "input_text", "text": "<environment_context>"}
    ]);
    assert!(!is_codex_injected_context(&mixed_image_and_context));
}

#[test]
fn subagent_notification_message_is_hidden_and_near_miss_is_shown() {
    let temp = Temp::new();
    let file_path = temp.0.join("rollout-subagent-notification.jsonl");

    let records = vec![
        json!({"type": "session_meta", "payload": {"id": THREAD_ID, "cwd": "/test/cwd"}}),
        // 1. Injected subagent_notification: must be HIDDEN
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-subagent-notif",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "<subagent_notification>{\"agent\":\"researcher\",\"status\":\"completed\"}</subagent_notification>"
                    }
                ]
            }
        }),
        // 2. Near-miss text "<subagent_notificationX": must be SHOWN
        json!({
            "type": "response_item",
            "payload": {
                "type": "message",
                "id": "msg-near-miss",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "<subagent_notificationX>Some user text</subagent_notificationX>"
                    }
                ]
            }
        }),
    ];

    let mut content = String::new();
    let mut offsets = Vec::new();
    let mut lengths = Vec::new();
    for r in &records {
        let line = format!("{}\n", r);
        offsets.push(content.len());
        lengths.push(line.len());
        content.push_str(&line);
    }
    fs::write(&file_path, &content).unwrap();

    let mut follower = Follower::with_content_for_agent(AgentType::Codex);
    assert!(follower.refresh(&file_path, THREAD_ID).unwrap());

    let transcript = &follower.transcript;

    // Only the near-miss message should be present; the exact subagent_notification is hidden
    assert_eq!(transcript.entries.len(), 1);
    assert_eq!(transcript.entries[0].id, "message:msg-near-miss");
    assert_eq!(transcript.entries[0].kind, "message");
    assert_eq!(transcript.entries[0].role.as_deref(), Some("user"));
    assert_eq!(
        transcript.entries[0].text,
        "<subagent_notificationX>Some user text</subagent_notificationX>"
    );
    assert_eq!(
        transcript.entries[0].record_refs,
        vec![format!("source:{}:{}", offsets[2], lengths[2])]
    );
}

#[test]
fn question_answer_retained_context_and_empty_messages_stay_hidden() {
    for preserve_content in [false, true] {
        let temp = Temp::new();
        let path = temp.0.join("question-answer.jsonl");
        let records = [
            json!({"type":"session_meta","payload":{"id":THREAD_ID,"cwd":"/test/cwd"}}),
            json!({"type":"response_item","payload":{"type":"function_call","id":"question-item","call_id":"question-call","name":"request_user_input","arguments":"{}"}}),
            json!({"type":"response_item","payload":{"type":"function_call_output","call_id":"question-call","output":"{\"answers\":{\"size\":{\"answers\":[\"None of the above\\nuser_note: Note\"]}}}"}}),
            json!({"type":"response_item","payload":{"type":"message","id":"assistant-answer","role":"assistant","content":[{"type":"output_text","text":"Thanks, I’ll use your answers."}]}}),
        ];
        fs::write(
            &path,
            records.iter().map(|r| format!("{r}\n")).collect::<String>(),
        )
        .unwrap();
        let mut follower = if preserve_content {
            Follower::with_content_for_agent(AgentType::Codex)
        } else {
            Follower::for_agent(AgentType::Codex)
        };
        follower.refresh(&path, THREAD_ID).unwrap();
        let previous = follower.transcript.entries.clone();
        assert_eq!(previous.len(), 2);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        let retained = json!({"type":"retained_context","payload":{"type":"verified_answer","turn_id":"turn-question","call_id":"question-call","questions":[{"question":"Pick a size","answer":"None of the above\nuser_note: Note"}],"acceptance_order":6}});
        writeln!(file, "{retained}").unwrap();
        for role in ["user", "assistant"] {
            for text in ["", " \t\r\n ", "\u{2003}"] {
                for id in ["empty-message", "assistant-answer"] {
                    let empty = json!({"type":"response_item","payload":{"type":"message","id":id,"role":role,"content":[{"type":"output_text","text":text,"annotations":[],"logprobs":[]}],"phase":"final_answer"}});
                    writeln!(file, "{empty}").unwrap();
                }
            }
        }
        file.flush().unwrap();
        follower.refresh(&path, THREAD_ID).unwrap();
        assert_eq!(follower.transcript.entries, previous);
        assert_eq!(follower.transcript.unknown_records, 0);
        writeln!(file, "{}", json!({"type":"response_item","payload":{"type":"message","id":"next-answer","role":"assistant","content":[{"type":"output_text","text":"I’ll continue with the plan."}],"phase":"final_answer"}})).unwrap();
        file.flush().unwrap();
        follower.refresh(&path, THREAD_ID).unwrap();
        assert_eq!(follower.transcript.entries.len(), 3);
        assert_eq!(
            follower.transcript.entries.last().unwrap().text,
            "I’ll continue with the plan."
        );
        assert_eq!(follower.transcript.unknown_records, 0);
    }
}
