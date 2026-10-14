use herdcats_bridge::{claude::Transcript, follower::Follower, resolver::Resolver};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
const SESSION: &str = "16246b9d-4c3d-54fc-b6d3-2a765d418491";
const FIXTURE: &str = include_str!("../../compatibility/phase0/fixtures/claude-transcript.jsonl");
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "hc-bridge-{}-{}",
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
fn message(id: &str, text: &str) -> Value {
    json!({"type":"user","uuid":id,"sessionId":SESSION,"message":{"role":"user","content":text}})
}
fn line(v: &Value) -> String {
    format!("{v}\n")
}

#[test]
fn captured_messages_and_tools_are_normalized_once_without_hook_attachments() {
    let mut transcript = Transcript::default();
    for _ in 0..2 {
        for line in FIXTURE.lines() {
            transcript
                .apply(&serde_json::from_str(line).unwrap(), SESSION)
                .unwrap();
        }
    }
    assert_eq!(
        transcript
            .entries
            .iter()
            .filter(|e| e.kind == "message")
            .count(),
        4
    );
    assert_eq!(
        transcript
            .entries
            .iter()
            .filter(|e| e.kind == "tool")
            .count(),
        2
    );
    let read = transcript
        .entries
        .iter()
        .find(|e| e.tool_name.as_deref() == Some("Read"))
        .unwrap();
    assert_eq!(read.status.as_deref(), Some("completed"));
    let question = transcript
        .entries
        .iter()
        .find(|e| e.tool_name.as_deref() == Some("AskUserQuestion"))
        .unwrap();
    assert_eq!(question.status.as_deref(), Some("failed"));
    assert!(transcript.entries.iter().all(|e| !e.text.contains("hook")));
}
#[test]
fn same_source_id_updates_text_instead_of_duplicating_it() {
    let mut t = Transcript::default();
    t.apply(&message("same", "first"), SESSION).unwrap();
    t.apply(&message("same", "second"), SESSION).unwrap();
    assert_eq!(t.entries.len(), 1);
    assert_eq!(t.entries[0].text, "second");
}
#[test]
fn foreign_sessions_fail_closed_and_sidechains_do_not_enter_main_timeline() {
    let mut t = Transcript::default();
    let mut v = message("one", "secret");
    v["sessionId"] = json!("another-session");
    assert_eq!(t.apply(&v, SESSION).unwrap_err(), "session_mismatch");
    v["sessionId"] = json!(SESSION);
    v["isSidechain"] = json!(true);
    t.apply(&v, SESSION).unwrap();
    assert!(t.entries.is_empty());
}
#[test]
fn unknown_records_are_counted_and_long_unicode_text_is_bounded() {
    let mut t = Transcript::default();
    t.apply(&json!({"type":"future_record"}), SESSION).unwrap();
    t.apply(&message("large", &"猫".repeat(10000)), SESSION)
        .unwrap();
    assert_eq!(t.unknown_records, 1);
    assert!(t.entries[0].truncated);
    assert!(t.entries[0].text.len() <= 8192);
    assert!(t.entries[0].text.is_char_boundary(t.entries[0].text.len()));
}
#[test]
fn resolver_uses_exact_session_name_and_rejects_ambiguous_or_escaping_paths() {
    let temp = Temp::new();
    let root = temp.0.join("projects");
    fs::create_dir_all(root.join("one")).unwrap();
    let target = root.join("one").join(format!("{SESSION}.jsonl"));
    fs::write(&target, FIXTURE).unwrap();
    fs::write(root.join("one/newest.jsonl"), "wrong").unwrap();
    let r = Resolver::new(&root).unwrap();
    assert_eq!(
        r.resolve("id", SESSION).unwrap().1,
        target.canonicalize().unwrap()
    );
    assert!(r.resolve("id", "../../outside").is_err());
    assert!(
        r.resolve("path", temp.0.join("outside.jsonl").to_str().unwrap())
            .is_err()
    );
    fs::create_dir(root.join("two")).unwrap();
    fs::write(root.join("two").join(format!("{SESSION}.jsonl")), FIXTURE).unwrap();
    assert_eq!(r.resolve("id", SESSION).unwrap_err(), "ambiguous_session");
}
#[test]
fn resolver_rejects_symlink_outside_allowed_root() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let root = temp.0.join("projects");
    fs::create_dir_all(root.join("one")).unwrap();
    let outside = temp.0.join(format!("{SESSION}.jsonl"));
    fs::write(&outside, FIXTURE).unwrap();
    symlink(&outside, root.join("one").join(format!("{SESSION}.jsonl"))).unwrap();
    assert!(
        Resolver::new(&root)
            .unwrap()
            .resolve("id", SESSION)
            .is_err()
    );
}
#[test]
fn follower_buffers_partial_utf8_and_resets_on_replace_truncate_or_rewrite() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let bytes = line(&message("one", "猫")).into_bytes();
    let split = bytes.iter().position(|b| *b == 0xe7).unwrap() + 1;
    fs::write(&path, &bytes[..split]).unwrap();
    let mut f = Follower::default();
    f.refresh(&path, SESSION).unwrap();
    assert!(f.transcript.entries.is_empty());
    fs::write(&path, &bytes).unwrap();
    assert!(f.refresh(&path, SESSION).unwrap());
    assert_eq!(f.transcript.entries[0].text, "猫");
    let generation = f.generation;
    let old = temp.0.join("old");
    fs::rename(&path, &old).unwrap();
    fs::write(&path, line(&message("two", "new"))).unwrap();
    f.refresh(&path, SESSION).unwrap();
    assert!(f.generation > generation);
    assert_eq!(f.transcript.entries.len(), 1);
    assert_eq!(f.transcript.entries[0].text, "new");
    fs::write(&path, b"").unwrap();
    f.refresh(&path, SESSION).unwrap();
    assert!(f.transcript.entries.is_empty());
    fs::write(&path, line(&message("two", "old"))).unwrap();
    f.refresh(&path, SESSION).unwrap();
    fs::write(&path, line(&message("two", "new"))).unwrap();
    f.refresh(&path, SESSION).unwrap();
    assert_eq!(f.transcript.entries[0].text, "new");
}
#[test]
fn follower_does_not_commit_malformed_or_foreign_complete_records() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    fs::write(&path, line(&message("one", "good"))).unwrap();
    let mut f = Follower::default();
    f.refresh(&path, SESSION).unwrap();
    fs::write(&path, format!("{}{{bad}}\n", line(&message("one", "good")))).unwrap();
    assert_eq!(f.refresh(&path, SESSION).unwrap_err(), "invalid_transcript");
    assert_eq!(f.transcript.entries.len(), 1);
    assert_eq!(f.transcript.entries[0].text, "good");
}
#[test]
fn oversized_source_identifiers_do_not_bypass_memory_and_frame_bounds() {
    let mut t = Transcript::default();
    assert_eq!(
        t.apply(&message(&"x".repeat(10000), "text"), SESSION)
            .unwrap_err(),
        "invalid_transcript"
    );
}
#[test]
fn evicted_duplicate_messages_and_tools_do_not_reenter_timeline() {
    let mut t = Transcript::default();
    let call = json!({"type":"assistant","uuid":"a","sessionId":SESSION,"message":{"content":[{"type":"tool_use","id":"t","name":"Read","input":{}}]}});
    t.apply(&call, SESSION).unwrap();
    for i in 0..1001 {
        t.apply(&message(&format!("m{i}"), "text"), SESSION)
            .unwrap();
    }
    let before = t.entries.clone();
    let dropped = t.dropped_entries;
    t.apply(&message("m0", "text"), SESSION).unwrap();
    t.apply(&call, SESSION).unwrap();
    assert!(t.entries == before, "evicted entries reentered timeline");
    assert_eq!(t.dropped_entries, dropped);
}
#[test]
fn replayed_tool_call_preserves_output_truncation() {
    let mut t = Transcript::default();
    let call = json!({"type":"assistant","uuid":"a","sessionId":SESSION,"message":{"content":[{"type":"tool_use","id":"t","name":"Read","input":{}}]}});
    t.apply(&call, SESSION).unwrap();
    t.apply(&json!({"type":"user","uuid":"r","sessionId":SESSION,"message":{"content":[{"type":"tool_result","tool_use_id":"t","content":"x".repeat(9000)}]}}),SESSION).unwrap();
    t.apply(&call, SESSION).unwrap();
    assert!(t.entries[0].truncated);
    assert_eq!(t.entries[0].status.as_deref(), Some("completed"));
}
#[test]
fn parent_directory_swap_cannot_read_outside_transcript() {
    use std::os::unix::fs::symlink;
    let temp = Temp::new();
    let root = temp.0.join("projects");
    fs::create_dir_all(root.join("one")).unwrap();
    let path = root.join("one").join(format!("{SESSION}.jsonl"));
    fs::write(&path, line(&message("a", "safe"))).unwrap();
    let resolver = Resolver::new(&root).unwrap();
    let (_, resolved) = resolver.resolve("id", SESSION).unwrap();
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(
        outside.join(format!("{SESSION}.jsonl")),
        line(&message("a", "outside")),
    )
    .unwrap();
    fs::rename(root.join("one"), root.join("old")).unwrap();
    symlink(&outside, root.join("one")).unwrap();
    assert!(resolver.open(&resolved).is_err());
    let mut follower = Follower::default();
    assert!(follower.refresh(&resolved, SESSION).is_err());
    assert!(follower.transcript.entries.is_empty());
}
#[test]
fn recognized_messages_with_invalid_content_fail_closed() {
    for content in [
        json!(null),
        json!({}),
        json!(12),
        json!([{"type":"text","text":12}]),
    ] {
        let mut t = Transcript::default();
        let mut row = message("a", "text");
        row["message"]["content"] = content;
        assert_eq!(t.apply(&row, SESSION).unwrap_err(), "invalid_transcript");
    }
}
#[test]
fn complete_and_partial_oversized_records_are_rejected() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    for ending in ["", "\n"] {
        fs::write(&path, format!("{}{}", "x".repeat(1024 * 1024 + 1), ending)).unwrap();
        assert_eq!(
            Follower::default().refresh(&path, SESSION).unwrap_err(),
            "record_too_large"
        );
    }
}
