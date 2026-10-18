use crate::{
    Result,
    claude::{Entry, Transcript, entry_id, is_routine_metadata},
};
use serde_json::value::RawValue;
use std::collections::BTreeMap;
fn object(raw: &str) -> Option<BTreeMap<String, &RawValue>> {
    serde_json::from_str(raw).ok()
}
fn string(raw: Option<&&RawValue>) -> Option<String> {
    raw.and_then(|r| serde_json::from_str(r.get()).ok())
}
fn preview(raw: &str) -> (String, bool) {
    let mut end = raw.len().min(8192);
    while !raw.is_char_boundary(end) {
        end -= 1;
    }
    (raw[..end].to_owned(), end < raw.len())
}
struct RecordContext<'a> {
    part: &'a [u8],
    offset: usize,
    reference: String,
    role: Option<String>,
    counter: usize,
}
impl RecordContext<'_> {
    fn reference_for(&self, raw: &str) -> Result<String> {
        let start = (raw.as_ptr() as usize)
            .checked_sub(self.part.as_ptr() as usize)
            .ok_or("invalid_transcript")?;
        if start
            .checked_add(raw.len())
            .is_none_or(|end| end > self.part.len())
        {
            return Err("invalid_transcript");
        }
        Ok(format!("source:{}:{}", self.offset + start, raw.len()))
    }
}
impl Transcript {
    pub(crate) fn attach_record(&mut self, id: &str, reference: &str) -> bool {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) {
            if entry.record_refs.iter().any(|r| r == reference) {
                return true;
            }
            if entry.record_refs.len() < 4 {
                entry.record_refs.push(reference.into());
                return true;
            }
        }
        false
    }
    fn content_bubble(
        &mut self,
        context: &mut RecordContext<'_>,
        label: &str,
        raw: &str,
        image: bool,
    ) -> Result<()> {
        let reference = context.reference_for(raw)?;
        let (details, truncated) = if image {
            (None, false)
        } else {
            let (s, t) = preview(raw);
            (Some(s), t)
        };
        let (text, label_truncated) = preview(label);
        let id = entry_id(
            "content",
            &format!("{}:{}", context.offset, context.counter),
        )?;
        context.counter += 1;
        self.upsert(Entry {
            id,
            kind: if image { "image" } else { "record" }.into(),
            text,
            role: context.role.clone(),
            tool_name: None,
            status: None,
            details,
            truncated: truncated || label_truncated,
            content_ref: Some(reference),
            record_refs: vec![context.reference.clone()],
        })
    }
    fn preserve_block(
        &mut self,
        raw: &RawValue,
        context: &mut RecordContext<'_>,
        depth: usize,
    ) -> Result<()> {
        let fields = object(raw.get()).unwrap_or_default();
        let kind = string(fields.get("type")).unwrap_or_else(|| "Unfamiliar content".into());
        if depth >= 16 {
            return self.content_bubble(context, &kind, raw.get(), false);
        }
        match kind.as_str() {
            "text" => {
                if string(fields.get("text")).is_none() {
                    self.content_bubble(context, "Unfamiliar text", raw.get(), false)?;
                }
            }
            "tool_use" | "tool_result" => {
                let key = if kind == "tool_use" {
                    "id"
                } else {
                    "tool_use_id"
                };
                let tool_id = string(fields.get(key));
                let tool_key = tool_id
                    .as_deref()
                    .and_then(|tid| entry_id("tool", tid).ok());
                let attached = tool_key
                    .as_ref()
                    .is_some_and(|k| self.attach_record(k, &context.reference));
                if !attached {
                    self.content_bubble(context, &kind, raw.get(), false)?;
                }
                if attached && kind == "tool_use" {
                    if let (Some(k), Some(input)) = (tool_key.as_ref(), fields.get("input")) {
                        let reference = context.reference_for(input.get())?;
                        if let Some(entry) = self.entries.iter_mut().find(|entry| &entry.id == k) {
                            entry.content_ref = Some(reference);
                        }
                    }
                }
                if kind == "tool_result" {
                    if let Some(content) = fields.get("content") {
                        if let Ok(blocks) = serde_json::from_str::<Vec<&RawValue>>(content.get()) {
                            for block in blocks {
                                self.preserve_block(block, context, depth + 1)?;
                            }
                        } else if serde_json::from_str::<String>(content.get()).is_err() {
                            self.content_bubble(context, "Tool result data", content.get(), false)?;
                        }
                    }
                }
            }
            "image" => self.content_bubble(context, "Image", raw.get(), true)?,
            _ => self.content_bubble(context, &kind, raw.get(), false)?,
        }
        Ok(())
    }
    pub fn preserve_record(&mut self, part: &[u8], session: &str, offset: usize) -> Result<()> {
        let raw: &RawValue = serde_json::from_slice(part).map_err(|_| "invalid_transcript")?;
        let fields = object(raw.get()).unwrap_or_default();
        if ["isSidechain", "isMeta"]
            .iter()
            .any(|flag| fields.get(*flag).is_some_and(|value| value.get() == "true"))
        {
            return Ok(());
        }
        if let Some(record_session) = string(fields.get("sessionId")) {
            if record_session != session {
                return Err("session_mismatch");
            }
        }
        let kind = string(fields.get("type")).unwrap_or_else(|| "Session record".into());
        if is_routine_metadata(&kind) {
            return Ok(());
        }
        let mut context = RecordContext {
            part,
            offset,
            reference: format!("source:{offset}:{}", part.len()),
            role: matches!(kind.as_str(), "user" | "assistant").then(|| kind.clone()),
            counter: 0,
        };
        if matches!(kind.as_str(), "user" | "assistant") {
            let attached = string(fields.get("uuid")).is_some_and(|id| {
                let msg_key = entry_id("message", &id).ok();
                let task_key = entry_id("task", &id).ok();
                msg_key
                    .as_ref()
                    .is_some_and(|k| self.attach_record(k, &context.reference))
                    || task_key
                        .as_ref()
                        .is_some_and(|k| self.attach_record(k, &context.reference))
            });
            if let Some(message) = fields.get("message").and_then(|m| object(m.get())) {
                if let Some(content) = message.get("content") {
                    if let Ok(blocks) = serde_json::from_str::<Vec<&RawValue>>(content.get()) {
                        for block in blocks {
                            self.preserve_block(block, &mut context, 0)?;
                        }
                    }
                }
            }
            if attached
                || context.counter > 0
                || self
                    .entries
                    .iter()
                    .any(|e| e.record_refs.contains(&context.reference))
            {
                return Ok(());
            }
        }
        self.content_bubble(&mut context, &kind, raw.get(), false)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const SESSION: &str = "session-a";
    #[test]
    fn images_and_unfamiliar_blocks_have_individual_lossless_references() {
        let row = json!({"type":"user","uuid":"m","sessionId":SESSION,"message":{"content":[
            {"type":"text","text":"See this"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":"YWJj"}},
            {"type":"future_block","nested":{"value":"never lose this"}}
        ]}});
        let bytes = serde_json::to_vec(&row).unwrap();
        let mut transcript = Transcript::default();
        transcript.apply(&row, SESSION).unwrap();
        transcript.preserve_record(&bytes, SESSION, 0).unwrap();
        let rows = serde_json::to_value(&transcript.entries).unwrap();
        let image = rows
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "image")
            .unwrap();
        assert!(
            image["content_ref"]
                .as_str()
                .unwrap()
                .starts_with("source:")
        );
        assert!(
            rows.as_array()
                .unwrap()
                .iter()
                .any(|r| r["kind"] == "record"
                    && r["text"].as_str().unwrap().contains("future_block"))
        );
        assert!(
            rows.as_array()
                .unwrap()
                .iter()
                .all(|r| r["record_refs"].as_array().unwrap().len() == 1)
        );
    }
    #[test]
    fn unfamiliar_top_level_records_are_visible_and_foreign_sessions_rejected() {
        let row =
            json!({"type":"future_record","sessionId":SESSION,"payload":{"secret":"preserved"}});
        let mut transcript = Transcript::default();
        transcript
            .preserve_record(&serde_json::to_vec(&row).unwrap(), SESSION, 10)
            .unwrap();
        assert_eq!(transcript.entries.len(), 1);
        assert_eq!(transcript.entries[0].kind, "record");
        let foreign = json!({"type":"future_record","sessionId":"foreign","payload":"private"});
        assert_eq!(
            transcript.preserve_record(&serde_json::to_vec(&foreign).unwrap(), SESSION, 100),
            Err("session_mismatch")
        );
    }
    #[test]
    fn unfamiliar_text_and_tool_result_shapes_have_their_own_bubbles() {
        let row = json!({"type":"user","uuid":"mixed","sessionId":SESSION,"message":{"content":[
            {"type":"text","text":{"future":"payload"}},
            {"type":"image","source":{"type":"base64","data":"YWJj"}},
            {"type":"tool_result","tool_use_id":"missing","content":{"future":"result"}}
        ]}});
        let bytes = serde_json::to_vec(&row).unwrap();
        let mut transcript = Transcript::default();
        transcript.preserve_record(&bytes, SESSION, 0).unwrap();
        assert!(
            transcript
                .entries
                .iter()
                .any(|e| e.kind == "record" && e.text == "Unfamiliar text")
        );
        assert!(
            transcript
                .entries
                .iter()
                .any(|e| e.kind == "record" && e.text == "Tool result data")
        );
        assert!(transcript.entries.iter().any(|e| e.kind == "image"));
    }
    #[test]
    fn excluded_records_never_publish_content_or_attach_to_main_entries() {
        for flag in ["isSidechain", "isMeta"] {
            for session in [SESSION, "foreign"] {
                let main = json!({"type":"assistant","uuid":"main","sessionId":SESSION,
                    "message":{"content":[{"type":"tool_use","id":"shared","name":"Read","input":{}}]}});
                let mut transcript = Transcript::default();
                transcript.apply(&main, SESSION).unwrap();
                let before = serde_json::to_value(&transcript.entries).unwrap();
                let mut excluded = json!({"type":"assistant","uuid":"main","sessionId":session,
                "message":{"content":[
                    {"type":"image","source":{"type":"base64","data":"private"}},
                    {"type":"tool_use","id":"shared","name":"Read","input":{"private":true}},
                    {"type":"future_block","private":true}
                ]}});
                excluded[flag] = json!(true);
                transcript
                    .preserve_record(&serde_json::to_vec(&excluded).unwrap(), SESSION, 50)
                    .unwrap();
                assert!(serde_json::to_value(&transcript.entries).unwrap() == before);
            }
        }
    }

    #[test]
    fn routine_metadata_does_not_evict_conversation_or_create_fallbacks() {
        let mut transcript = Transcript::default();
        let message = json!({"type":"user","uuid":"keep","sessionId":SESSION,"message":{"content":"Keep me"}});
        transcript.apply(&message, SESSION).unwrap();
        let before = serde_json::to_value(&transcript.entries).unwrap();
        for i in 0..1100 {
            for kind in [
                "attachment",
                "system",
                "file-history-snapshot",
                "last-prompt",
                "ai-title",
                "mode",
                "permission-mode",
                "atis-latch",
            ] {
                let metadata = json!({"type":kind,"sessionId":SESSION,"payload":i});
                transcript.apply(&metadata, SESSION).unwrap();
                transcript
                    .preserve_record(&serde_json::to_vec(&metadata).unwrap(), SESSION, i)
                    .unwrap();
            }
        }
        assert!(serde_json::to_value(&transcript.entries).unwrap() == before);
        assert_eq!(transcript.dropped_entries, 0);
        assert_eq!(transcript.unknown_records, 0);
    }

    #[test]
    fn oversized_images_are_referenced_not_embedded_in_live_frames() {
        let row = json!({"type":"assistant","uuid":"large","sessionId":SESSION,"message":{"content":[
            {"type":"image","source":{"type":"base64","data":"A".repeat(2_000_000)}}
        ]}});
        let bytes = serde_json::to_vec(&row).unwrap();
        let mut transcript = Transcript::default();
        transcript.apply_oversized(&bytes, SESSION).unwrap();
        transcript.preserve_record(&bytes, SESSION, 0).unwrap();
        assert!(transcript.entries.iter().any(|e| e.kind == "image"));
        assert!(serde_json::to_vec(&transcript.entries).unwrap().len() < 32_768);
    }
}
