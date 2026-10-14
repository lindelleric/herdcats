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
fn complete_oversized_image_records_are_counted_once_and_later_text_is_visible() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let image = oversized_image_record();
    fs::write(
        &path,
        format!("{}{}", line(&message("before", "before image")), image),
    )
    .unwrap();
    let mut follower = Follower::default();
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.unknown_records, 1);
    assert_eq!(follower.transcript.entries.len(), 1);
    assert!(!follower.refresh(&path, SESSION).unwrap());
    append(&path, &line(&message("after", "after image")));
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.unknown_records, 1);
    assert_eq!(follower.transcript.entries.len(), 2);
    assert_eq!(follower.transcript.entries[1].text, "after image");
    assert_eq!(follower.offset, fs::metadata(&path).unwrap().len());
}

#[test]
fn oversized_record_with_text_and_image_preserves_text_and_counts_omission() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let rec = line(&json!({
        "type": "user",
        "uuid": "u-text-img",
        "sessionId": SESSION,
        "message": {
            "role": "user",
            "content": [
                {"type": "text", "text": "Why is this button misaligned?"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "A".repeat(1024 * 1024)}}
            ]
        }
    }));
    fs::write(&path, &rec).unwrap();
    let mut follower = Follower::default();
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.entries.len(), 1);
    assert_eq!(
        follower.transcript.entries[0].text,
        "Why is this button misaligned?"
    );
    assert_eq!(follower.transcript.entries[0].kind, "message");
    assert_eq!(follower.transcript.unknown_records, 1);
}

#[test]
fn tool_use_followed_by_oversized_image_tool_result_completes_tool() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let tool_call = line(&json!({
        "type": "assistant",
        "uuid": "a-tool-call",
        "sessionId": SESSION,
        "message": {
            "role": "assistant",
            "content": [
                {"type": "tool_use", "id": "t1", "name": "Read", "input": {"path": "screenshot.png"}}
            ]
        }
    }));
    let tool_res = line(&json!({
        "type": "user",
        "uuid": "u-tool-res",
        "sessionId": SESSION,
        "message": {
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "t1",
                    "content": [
                        {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "A".repeat(1024 * 1024)}}
                    ],
                    "is_error": false
                }
            ]
        }
    }));
    fs::write(&path, format!("{tool_call}{tool_res}")).unwrap();
    let mut follower = Follower::default();
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.entries.len(), 1);
    assert_eq!(follower.transcript.entries[0].id, "tool:t1");
    assert_eq!(
        follower.transcript.entries[0].status.as_deref(),
        Some("completed")
    );
    assert_eq!(follower.transcript.unknown_records, 1);
}

#[test]
fn foreign_session_oversized_record_fails_closed() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let image = line(&json!({
        "type": "user",
        "uuid": "foreign-img",
        "sessionId": "foreign-session-uuid",
        "message": {
            "role": "user",
            "content": [
                {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "A".repeat(1024 * 1024)}}
            ]
        }
    }));
    let before = line(&message("before", "before image"));
    fs::write(&path, format!("{before}{image}")).unwrap();
    let mut follower = Follower::default();
    assert_eq!(
        follower.refresh(&path, SESSION).unwrap_err(),
        "session_mismatch"
    );
    assert!(follower.transcript.entries.is_empty());
}

#[test]
fn incomplete_oversized_image_records_wait_for_newline_without_counting() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let image = oversized_image_record();
    let before = line(&message("before", "before image"));
    fs::write(&path, format!("{}{}", before, &image[..image.len() - 2])).unwrap();
    let mut follower = Follower::default();
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.offset, before.len() as u64);
    assert_eq!(follower.transcript.unknown_records, 0);
    assert!(!follower.refresh(&path, SESSION).unwrap());
    append(&path, "}");
    assert!(!follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.offset, before.len() as u64);
    assert_eq!(follower.transcript.unknown_records, 0);
    append(
        &path,
        &format!("\n{}", line(&message("after", "after image"))),
    );
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.unknown_records, 1);
    assert_eq!(follower.transcript.entries.len(), 2);
    assert_eq!(follower.transcript.entries[1].text, "after image");
    assert!(!follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.unknown_records, 1);
}

#[test]
fn oversized_record_omissions_reset_on_rewrite_and_small_malformed_rows_fail_closed() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let image = oversized_image_record();
    fs::write(
        &path,
        format!("{}{}", image, line(&message("old", "old text"))),
    )
    .unwrap();
    let mut follower = Follower::default();
    follower.refresh(&path, SESSION).unwrap();
    let generation = follower.generation;
    fs::write(
        &path,
        format!("{}{}", image, line(&message("new", "new text"))),
    )
    .unwrap();
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.generation, generation + 1);
    assert_eq!(follower.transcript.unknown_records, 1);
    assert_eq!(follower.transcript.entries.len(), 1);
    assert_eq!(follower.transcript.entries[0].text, "new text");
    let offset = follower.offset;
    append(&path, &format!("{}{{bad}}\n", image));
    assert_eq!(
        follower.refresh(&path, SESSION).unwrap_err(),
        "invalid_transcript"
    );
    assert_eq!(follower.offset, offset);
    assert_eq!(follower.transcript.unknown_records, 1);
    assert_eq!(follower.transcript.entries[0].text, "new text");
    fs::write(&path, line(&message("small", "small text"))).unwrap();
    follower.refresh(&path, SESSION).unwrap();
    assert_eq!(follower.transcript.unknown_records, 0);
    assert_eq!(follower.transcript.entries.len(), 1);
    assert_eq!(follower.transcript.entries[0].text, "small text");
}

fn oversized_image_record() -> String {
    line(
        &json!({"type":"user","uuid":"image-result","sessionId":SESSION,
        "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"image-tool",
            "content":[{"type":"image","source":{"type":"base64","media_type":"image/png",
                "data":"A".repeat(1024 * 1024)}}]}]}}),
    )
}

fn append(path: &std::path::Path, bytes: &str) {
    use std::io::Write;
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(bytes.as_bytes())
        .unwrap();
}

#[test]
fn unknown_progress_content_object_tolerated_in_normal_and_oversized() {
    let mut t = Transcript::default();
    let row = json!({
        "type": "assistant",
        "uuid": "prog-normal",
        "sessionId": SESSION,
        "message": {
            "role": "assistant",
            "content": [
                {"type": "progress", "content": {"stage": "thinking", "depth": 3}}
            ]
        }
    });
    assert!(t.apply(&row, SESSION).is_ok());

    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let oversized_prog = line(&json!({
        "type": "assistant",
        "uuid": "prog-oversized",
        "sessionId": SESSION,
        "message": {
            "role": "assistant",
            "content": [
                {
                    "type": "progress",
                    "content": {"stage": "thinking", "payload": "P".repeat(1024 * 1024 + 10)}
                }
            ]
        }
    }));
    fs::write(&path, oversized_prog).unwrap();
    let mut follower = Follower::default();
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.unknown_records, 1);
    assert_eq!(follower.transcript.entries.len(), 0);
}

#[test]
fn web_search_tool_result_error_object_tolerated_in_normal_and_oversized() {
    let mut t = Transcript::default();
    let row = json!({
        "type": "assistant",
        "uuid": "search-normal",
        "sessionId": SESSION,
        "message": {
            "role": "assistant",
            "content": [
                {"type": "web_search_tool_result", "error": {"code": 500, "message": "fail"}}
            ]
        }
    });
    assert!(t.apply(&row, SESSION).is_ok());

    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let oversized_search = line(&json!({
        "type": "assistant",
        "uuid": "search-oversized",
        "sessionId": SESSION,
        "message": {
            "role": "assistant",
            "content": [
                {
                    "type": "web_search_tool_result",
                    "error": {"code": 500, "message": "S".repeat(1024 * 1024 + 10)}
                }
            ]
        }
    }));
    fs::write(&path, oversized_search).unwrap();
    let mut follower = Follower::default();
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.unknown_records, 1);
    assert_eq!(follower.transcript.entries.len(), 0);
}

#[test]
fn tool_result_content_string_array_tolerated_in_normal_and_oversized() {
    let mut t = Transcript::default();
    let call = json!({
        "type": "assistant",
        "uuid": "call-1",
        "sessionId": SESSION,
        "message": {
            "role": "assistant",
            "content": [{"type": "tool_use", "id": "t1", "name": "shell", "input": {}}]
        }
    });
    t.apply(&call, SESSION).unwrap();
    let res = json!({
        "type": "user",
        "uuid": "res-1",
        "sessionId": SESSION,
        "message": {
            "role": "user",
            "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": ["line 1", "line 2"]}
            ]
        }
    });
    assert!(t.apply(&res, SESSION).is_ok());
    assert_eq!(t.entries[0].status.as_deref(), Some("completed"));

    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");
    let call_over = line(&json!({
        "type": "assistant",
        "uuid": "call-over",
        "sessionId": SESSION,
        "message": {
            "role": "assistant",
            "content": [{"type": "tool_use", "id": "t2", "name": "shell", "input": {}}]
        }
    }));
    let res_over = line(&json!({
        "type": "user",
        "uuid": "res-over",
        "sessionId": SESSION,
        "message": {
            "role": "user",
            "content": [
                {
                    "type": "tool_result",
                    "tool_use_id": "t2",
                    "content": ["line 1", "A".repeat(1024 * 1024 + 10)]
                }
            ]
        }
    }));
    fs::write(&path, format!("{call_over}{res_over}")).unwrap();
    let mut follower = Follower::default();
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.entries.len(), 1);
    assert_eq!(follower.transcript.entries[0].status.as_deref(), Some("completed"));
}

#[test]
fn oversized_text_only_message_and_tool_input_do_not_count_omission() {
    let temp = Temp::new();
    let path = temp.0.join("log.jsonl");

    // Pure oversized text message (> 1MB)
    let big_text = "Words in a long document. ".repeat(45_000);
    let msg = line(&json!({
        "type": "user",
        "uuid": "big-text",
        "sessionId": SESSION,
        "message": {
            "role": "user",
            "content": big_text
        }
    }));
    fs::write(&path, &msg).unwrap();
    let mut follower = Follower::default();
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.unknown_records, 0, "text-only oversized message must not count as omission");
    assert_eq!(follower.transcript.entries.len(), 1);
    assert!(follower.transcript.entries[0].truncated);
    assert_eq!(follower.transcript.entries[0].text.len(), 8192);

    // Oversized tool_use input (> 1MB)
    let tool_call = line(&json!({
        "type": "assistant",
        "uuid": "big-tool",
        "sessionId": SESSION,
        "message": {
            "role": "assistant",
            "content": [
                {
                    "type": "tool_use",
                    "id": "t-big",
                    "name": "large_input",
                    "input": {"big_param": "X".repeat(1024 * 1024 + 10)}
                }
            ]
        }
    }));
    append(&path, &tool_call);
    assert!(follower.refresh(&path, SESSION).unwrap());
    assert_eq!(follower.transcript.unknown_records, 0, "fully represented tool input must not count as omission");
    assert_eq!(follower.transcript.entries.len(), 2);
    assert!(follower.transcript.entries[1].truncated);
}
