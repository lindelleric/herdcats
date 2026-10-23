use crate::{
    Result,
    adapter::Adapter,
    approval_match::{self, PendingRequest, Resolution},
    approval_screen::{self, ApprovalDialog, OptionKey},
    claude::{ClaudeAdapter, Entry},
    codex::{CodexAdapter, CodexResolver},
    follower::{Follower, MAX_RECORD},
    herdr::{Client, Pane},
    hooks::Agent,
    model_control::{self, ClaudeConfirmation, PickerScreen},
    resolver::Resolver,
    question_match::{self, PendingQuestion, QuestionKind},
    question_keys::{self, ExpectedScreen, Input, Step},
    question_screen::{self, QuestionScreen, QuestionStep},
    spool,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
struct InteractionError {
    code: &'static str,
    steps_sent: Option<usize>,
}
impl From<&'static str> for InteractionError {
    fn from(code: &'static str) -> Self { Self { code, steps_sent: None } }
}
impl InteractionError {
    fn stale_after(steps_sent: usize) -> Self { Self { code: "stale", steps_sent: Some(steps_sent) } }
    fn data(&self) -> Value {
        let mut data = json!({"code":self.code});
        if let Some(steps_sent) = self.steps_sent { data["steps_sent"] = json!(steps_sent); }
        data
    }
}
type InteractionResult<T> = std::result::Result<T, InteractionError>;

/// One successful poll: the verified identity, whether the timeline reset, the
/// agent status, and descriptor clones for interaction/model tail reads.
struct PollOutcome {
    identity: Identity,
    reset: bool,
    status: String,
    approval_file: Option<std::fs::File>,
    model_file: Option<std::fs::File>,
}

#[derive(Clone, PartialEq, Serialize)]
struct Identity {
    pane_id: String,
    terminal_id: String,
    agent: String,
    native_session_id: String,
}
struct PendingApproval {
    identity: Identity,
    generation: u64,
    request: PendingRequest,
    dialog: ApprovalDialog,
}
impl PendingApproval {
    fn response_key(&self, params: &Value) -> Result<String> {
        let number = params["option"].as_u64().ok_or("mismatch")?;
        let option = self.dialog.options.iter()
            .find(|option| u64::from(option.number) == number).ok_or("mismatch")?;
        let rows = params["label_lines"].as_array().ok_or("mismatch")?;
        if rows.len() != option.label_lines.len()
            || !rows.iter().zip(&option.label_lines).all(|(row, label)| row.as_str() == Some(label))
        {
            return Err("mismatch");
        }
        match (self.request.agent, &option.key) {
            (Agent::Claude, OptionKey::Digit(n)) if *n > 0 => Ok(n.to_string()),
            (Agent::Codex, OptionKey::Letter(c)) if c.is_ascii_alphabetic() => Ok(c.to_string()),
            (Agent::Codex, OptionKey::Escape) => Ok("esc".into()),
            _ => Err("mismatch"),
        }
    }
    fn data(&self) -> Value {
        let options: Vec<_> = self.dialog.options.iter().map(|option| {
            let key = match option.key {
                OptionKey::Digit(n) => n.to_string(),
                OptionKey::Letter(c) => c.to_string(),
                OptionKey::Escape => "esc".into(),
            };
            json!({"number":option.number,"label_lines":option.label_lines,"key":key})
        }).collect();
        json!({
            "identity":self.identity,"generation":self.generation,"kind":"approval",
            "request_id":self.request.request_id,"agent":self.request.agent.name(),
            "tool_name":self.request.tool_name,"command":self.request.tool_input["command"],
            "description":if self.request.agent == Agent::Claude {
                self.request.tool_input.get("description").and_then(Value::as_str)
            } else { None },
            "justification":self.request.justification,"environment":self.dialog.environment,
            "title":self.dialog.title,"options":options,"selected":self.dialog.selected
        })
    }
}
struct PendingQuestionEvent {
    identity: Identity,
    generation: u64,
    request: PendingQuestion,
    screen: QuestionScreen,
    fresh: bool,
}
enum PendingInteraction {
    Approval(PendingApproval),
    Question(PendingQuestionEvent),
}
impl PendingInteraction {
    fn data(&self) -> Value {
        match self {
            Self::Approval(p) => p.data(),
            Self::Question(p) => {
                let questions: Vec<_> = p.request.questions.iter().enumerate().map(|(index, q)| {
                    let mut options: Vec<_> = q.options.iter().enumerate().map(|(i, o)|
                        json!({"number":i + 1,"label":o.label,"description":o.description})).collect();
                    if p.request.kind == QuestionKind::CodexPlan && q.allows_other {
                        options.push(json!({"number":options.len() + 1,"label":"None of the above",
                            "description":"Optionally, add details in notes (tab)"}));
                    }
                    json!({"index":index,"header":q.header,"text":q.text,"options":options,
                        "multi_select":q.multi_select,"allows_other":q.allows_other})
                }).collect();
                let step = match p.screen.step {
                    QuestionStep::ClaudeQuestion(index) => json!({"kind":"claude_question","index":index}),
                    QuestionStep::ClaudeReview => json!({"kind":"claude_review"}),
                    QuestionStep::CodexQuestion { index, total } => json!({"kind":"codex_question","index":index,"total":total}),
                    QuestionStep::AsyncQueued => json!({"kind":"async_queued"}),
                    QuestionStep::AsyncPicker(index) => json!({"kind":"async_picker","index":index}),
                };
                let kind = match p.request.kind {
                    QuestionKind::ClaudeAsk => "claude", QuestionKind::CodexPlan => "codex_plan",
                    QuestionKind::CodexAsync => "codex_async",
                };
                json!({"identity":p.identity,"generation":p.generation,"request_id":p.request.request_id,
                    "agent":p.identity.agent,"kind":"question","question_kind":kind,
                    "questions":questions,"step":step,"fresh":p.fresh,"countdown":p.screen.countdown})
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum QuestionInputStamp {
    Claude(ApprovalInputStamp),
    Codex(ApprovalFileStamp),
}
struct QuestionMatchCache {
    identity: Identity,
    stamp: QuestionInputStamp,
    request: Option<PendingQuestion>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ApprovalFileStamp {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}
impl ApprovalFileStamp {
    fn of(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt;
        Self {
            dev: metadata.dev(), ino: metadata.ino(), len: metadata.len(),
            mtime: (metadata.mtime(), metadata.mtime_nsec()),
            ctime: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ApprovalInputStamp {
    spool: ApprovalFileStamp,
    transcript: ApprovalFileStamp,
}
struct ApprovalMatchCache {
    identity: Identity,
    stamp: ApprovalInputStamp,
    request: Option<PendingRequest>,
}
type ModelPair = Option<(String, Option<String>)>;

struct ModelOverride {
    applied: ModelPair,
    applied_label: Option<String>,
    scanned_at_set: ModelPair,
}

impl ModelOverride {
    fn label_for(&self, pair: &ModelPair) -> Option<String> {
        (self.applied == *pair)
            .then(|| self.applied_label.clone())
            .flatten()
    }
    fn resolve(slot: &mut Option<Self>, scanned: ModelPair) -> ModelPair {
        if let Some(current) = slot {
            if current.scanned_at_set == scanned {
                return current.applied.clone();
            }
            *slot = None;
        }
        scanned
    }
}

struct Watch {
    pane_id: String,
    identity: Option<Identity>,
    follower: Follower,
    generation: u64,
    previous: Vec<Entry>,
    unavailable: Option<&'static str>,
    status: String,
    unknown_records: usize,
    dropped_entries: usize,
    source_changing_count: u32,
    delayed: bool,
    agent: Option<String>,
    // What was last EMITTED as the unavailable state, so a change of any
    // component re-emits: the code, the occupant agent, and the Codex daemon
    // status. Without the agent component, a pane that swaps its occupant
    // codex <-> claude while both fail with the same no-session code would
    // keep the stale event (including the Codex-only codex_daemon field) and
    // would not reset the generation for the new occupant.
    unavailable_agent: Option<String>,
    last_daemon_status: Option<&'static str>,
    pending: Option<PendingInteraction>,
    approval_cache: Option<ApprovalMatchCache>,
    question_cache: Option<QuestionMatchCache>,
    source_pane: Option<Pane>,
    // Transcript stamp behind the last model/effort scan (or emission reset),
    // so unchanged files skip tail reads on every poll.
    model_stamp: Option<(u64, (i64, i64))>,
    last_model: Option<(String, Option<String>)>,
    model_override: Option<ModelOverride>,
}
pub struct Service {
    pub client: Client,
    pub resolver: Option<Resolver>,
    pub codex_resolver: Option<CodexResolver>,
    adapters: Vec<Box<dyn Adapter>>,
    epoch: String,
    sequence: u64,
    generation: u64,
    initialized: bool,
    include_content: bool,
    include_approvals: bool,
    include_questions: bool,
    watch: Option<Watch>,
    codex_daemon_cache: Option<(std::time::Instant, &'static str)>,
    // The CONFIGURED (never canonicalized) Codex home. CodexResolver::new
    // canonicalizes the sessions root, so deriving the home from
    // `codex_resolver.root.parent()` would follow a symlinked sessions dir
    // (e.g. ~/.codex/sessions -> another volume) and check the WRONG
    // app-server-daemon/daemon.pid — reporting not_running while the real
    // daemon runs under the configured home.
    codex_home: Option<PathBuf>,
    spool_dir: Option<PathBuf>,
    include_model_control: bool,
    // settings.json beside the configured (non-canonicalized) Claude root's
    // parent; `~/.claude/projects` -> `~/.claude/settings.json`.
    claude_settings_path: Option<PathBuf>,
}
pub fn page(entries: &[Entry], before: usize) -> Value {
    let mut start = before.saturating_sub(100);
    // Leave room for envelope and identity; JSON escaping can expand source text.
    while start < before
        && serde_json::to_vec(&entries[start..before]).map_or(true, |v| v.len() > MAX_RECORD / 2)
    {
        start += 1;
    }
    json!({"entries":entries[start..before], "has_more":start>0,
        "before":entries.get(start).filter(|_|start<before).map(|e|e.id.as_str())})
}
impl Service {
    pub fn new(socket: PathBuf, root: PathBuf) -> Self {
        let resolver = Resolver::new(&root).ok();
        let codex_home = crate::codex_config::resolve_codex_dir();
        let codex_root = codex_home.clone().map(|p| p.join("sessions"));
        let codex_resolver = codex_root.and_then(|r| CodexResolver::new(&r).ok());
        let mut service = Self::with_resolvers(socket, resolver, codex_resolver);
        service.codex_home = codex_home;
        service.claude_settings_path = root
            .parent()
            .map(|parent| parent.join("settings.json"))
            .filter(|path| path.is_absolute());
        service
    }

    pub fn codex_root_default() -> Option<PathBuf> {
        crate::codex_config::resolve_codex_dir().map(|p| p.join("sessions"))
    }

    pub fn codex_root_default_from(
        codex_home: Option<impl AsRef<Path>>,
        home: Option<impl AsRef<Path>>,
    ) -> Option<PathBuf> {
        crate::codex_config::resolve_codex_dir_from(codex_home, home).map(|p| p.join("sessions"))
    }
    pub fn with_resolvers(
        socket: PathBuf,
        resolver: Option<Resolver>,
        codex_resolver: Option<CodexResolver>,
    ) -> Self {
        let epoch = format!(
            "{}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            std::process::id()
        );
        let adapters: Vec<Box<dyn Adapter>> = vec![
            Box::new(ClaudeAdapter::new(resolver.clone())),
            Box::new(CodexAdapter::new(codex_resolver.clone())),
        ];
        Self {
            client: Client { socket },
            resolver,
            codex_resolver,
            adapters,
            epoch,
            sequence: 0,
            generation: 0,
            initialized: false,
            include_content: false,
            include_approvals: false,
            include_questions: false,
            watch: None,
            codex_daemon_cache: None,
            codex_home: None,
            spool_dir: spool::resolve_spool_dir(),
            include_model_control: false,
            claude_settings_path: None,
        }
    }
    pub fn invalidate_codex_daemon_cache(&mut self) {
        self.codex_daemon_cache = None;
    }
    fn cached_codex_daemon_status(&mut self) -> &'static str {
        if let Some((cached_at, status)) = self.codex_daemon_cache {
            // Monotonic clock: wall-clock jumps must not extend or shrink the TTL.
            if cached_at.elapsed() < Duration::from_secs(3) {
                return status;
            }
        }
        // Resolve from the CONFIGURED home, not the canonicalized resolver
        // root's parent: a symlinked sessions dir must not redirect the
        // daemon pid lookup to the symlink target's parent.
        let codex_dir = self
            .codex_home
            .clone()
            .or_else(crate::codex_config::resolve_codex_dir);
        let status = match codex_dir {
            Some(dir) => {
                let pid_file = dir.join("app-server-daemon").join("daemon.pid");
                match crate::codex_config::check_process(&pid_file, &dir) {
                    crate::codex_config::ProcessStatus::Running => "running",
                    crate::codex_config::ProcessStatus::NotRunning => "not_running",
                    crate::codex_config::ProcessStatus::Unknown => "unknown",
                }
            }
            None => "not_running",
        };
        self.codex_daemon_cache = Some((std::time::Instant::now(), status));
        status
    }
    #[doc(hidden)]
    pub fn with_codex_home(mut self, home: PathBuf) -> Self {
        self.codex_home = Some(home);
        self.invalidate_codex_daemon_cache();
        self
    }
    #[doc(hidden)]
    pub fn with_claude_settings(mut self, path: PathBuf) -> Self {
        self.claude_settings_path = Some(path);
        self
    }
    pub fn with_codex_root(mut self, root: PathBuf) -> Self {
        self.codex_resolver = CodexResolver::new(&root).ok();
        self.adapters = vec![
            Box::new(ClaudeAdapter::new(self.resolver.clone())),
            Box::new(CodexAdapter::new(self.codex_resolver.clone())),
        ];
        self
    }
    fn adapter_for(&self, agent: &str) -> Option<&dyn Adapter> {
        self.adapters
            .iter()
            .find(|a| a.supports(agent))
            .map(|a| a.as_ref())
    }
    fn write(out: &mut impl Write, frame: &Value) -> io::Result<()> {
        let bytes = serde_json::to_vec(frame)?;
        if bytes.len() > MAX_RECORD {
            return Err(io::Error::other("output_frame_too_large"));
        }
        out.write_all(&bytes)?;
        out.write_all(b"\n")?;
        out.flush()
    }
    fn error(out: &mut impl Write, id: Value, code: &str) -> io::Result<()> {
        Self::write(out, &json!({"id":id,"error":{"code":code}}))
    }
    fn event(&mut self, out: &mut impl Write, event: &str, data: Value) -> io::Result<()> {
        self.sequence += 1;
        Self::write(
            out,
            &json!({"event":event,"epoch":self.epoch,"seq":self.sequence,"data":data}),
        )
    }
    fn poll_inner(
        &self,
        watch: &mut Watch,
        refresh: &mut impl FnMut(&mut Follower, std::fs::File, &str) -> Result<bool>,
    ) -> Result<PollOutcome> {
        let snapshot = self.client.snapshot()?;
        let pane = snapshot
            .panes
            .iter()
            .find(|p| p.pane_id == watch.pane_id)
            .ok_or("pane_unavailable")?;
        watch.agent = pane.agent.clone();
        let agent = pane.agent.as_deref().ok_or("unsupported_agent")?;
        let adapter = self.adapter_for(agent).ok_or("unsupported_agent")?;
        let (session, file) = adapter.resolve_and_open(pane)?;
        if pane.terminal_id.is_empty()
            || pane.terminal_id.len() > 256
            || pane.agent_status.len() > 256
        {
            return Err("invalid_herdr_response");
        }
        let identity = Identity {
            pane_id: pane.pane_id.clone(),
            terminal_id: pane.terminal_id.clone(),
            agent: adapter.name().into(),
            native_session_id: session.clone(),
        };
        if watch.identity.as_ref() != Some(&identity) {
            watch.follower = adapter.new_follower(self.include_content);
            // A transient read of a new occupant cannot authorize serving the
            // previous occupant's cached transcript.
            watch.identity = None;
            watch.model_override = None;
            watch.approval_cache = None;
            watch.question_cache = None;
        }
        if pane.agent_status != "blocked" {
            watch.approval_cache = None;
            watch.question_cache = None;
        }
        watch.source_pane = Some(pane.clone());
        // Clone only the descriptor. Tail reading happens after timeline work,
        // using the same adapter-verified source and no additional path lookup.
        let approval_file = if (self.include_approvals || self.include_questions) && pane.agent_status == "blocked" {
            file.try_clone().ok()
        } else {
            None
        };
        let model_file = if self.include_model_control { file.try_clone().ok() } else { None };
        refresh(&mut watch.follower, file, &session)?;
        let reset = watch.identity.as_ref() != Some(&identity)
            || watch.unavailable.is_some()
            || watch.follower.generation != watch.generation;
        Ok(PollOutcome {
            identity,
            reset,
            status: pane.agent_status.clone(),
            approval_file,
            model_file,
        })
    }
    pub fn poll(&mut self, out: &mut impl Write) -> io::Result<()> {
        self.poll_with_refresh(out, |follower, file, session| {
            follower.refresh_file(file, session)
        })
    }
    fn poll_with_refresh(
        &mut self,
        out: &mut impl Write,
        mut refresh: impl FnMut(&mut Follower, std::fs::File, &str) -> Result<bool>,
    ) -> io::Result<()> {
        let Some(mut watch) = self.watch.take() else {
            return Ok(());
        };
        let mut pending = None;
        let mut preserve_pending = false;
        match self.poll_inner(&mut watch, &mut refresh) {
            Err("source_changing") if watch.identity.is_some() && watch.unavailable.is_none() => {
                preserve_pending = true;
                watch.source_changing_count += 1;
                if watch.source_changing_count >= 10 && !watch.delayed {
                    watch.delayed = true;
                    let identity = watch.identity.clone().unwrap();
                    let status = watch.status.clone();
                    self.event(
                        out,
                        "timeline.updated",
                        json!({
                            "identity": identity,
                            "generation": self.generation,
                            "entries": [],
                            "evicted_ids": [],
                            "agent_status": status,
                            "unknown_records": watch.unknown_records,
                            "dropped_entries": watch.dropped_entries,
                            "delayed": true
                        }),
                    )?;
                }
            }
            Err(code) => {
                watch.source_pane = None;
                watch.approval_cache = None;
                watch.question_cache = None;
                watch.source_changing_count = 0;
                watch.delayed = false;
                let is_codex_no_session = watch.agent.as_deref() == Some("codex")
                    && matches!(
                        code,
                        "session_reference_unavailable"
                            | "ambiguous_session"
                            | "invalid_session_reference"
                    );
                let current_daemon_status = if is_codex_no_session {
                    Some(self.cached_codex_daemon_status())
                } else {
                    None
                };

                // Emit whenever the emitted-unavailable triple changed: the
                // code, the occupant agent, or the daemon status. An agent
                // change goes through the same generation/reset path below as
                // any other occupant change.
                let should_emit = watch.unavailable != Some(code)
                    || watch.unavailable_agent != watch.agent
                    || watch.last_daemon_status != current_daemon_status;

                if should_emit {
                    self.generation += 1;
                    watch.identity = None;
                    watch.model_override = None;
                    watch.previous.clear();
                    watch.follower = if self.include_content {
                        Follower::with_content()
                    } else {
                        Follower::default()
                    };
                    watch.unavailable = Some(code);
                    watch.unavailable_agent = watch.agent.clone();
                    watch.last_daemon_status = current_daemon_status;
                    let mut data = json!({
                        "pane_id": watch.pane_id,
                        "generation": self.generation,
                        "code": code
                    });
                    if let Some(daemon_status) = current_daemon_status {
                        data["codex_daemon"] = json!(daemon_status);
                    }
                    self.event(out, "timeline.unavailable", data)?;
                }
            }
            Ok(PollOutcome {
                identity,
                reset,
                status,
                approval_file,
                model_file,
            }) => {
                let was_delayed = watch.delayed;
                watch.source_changing_count = 0;
                watch.delayed = false;
                let data = &watch.follower.transcript;
                let evicted: Vec<_> = watch
                    .previous
                    .iter()
                    .filter(|old| !data.entries.iter().any(|new| new.id == old.id))
                    .map(|old| old.id.clone())
                    .collect();
                if reset {
                    watch.approval_cache = None;
                    watch.question_cache = None;
                    watch.last_model = None;
                    watch.model_override = None;
                    watch.model_stamp = None;
                    self.generation += 1;
                    let mut body = page(&data.entries, data.entries.len());
                    body["identity"] = json!(identity);
                    body["generation"] = json!(self.generation);
                    body["agent_status"] = json!(status);
                    body["unknown_records"] = json!(data.unknown_records);
                    body["dropped_entries"] = json!(data.dropped_entries);
                    body["delayed"] = json!(false);
                    self.event(out, "timeline.snapshot", body)?;
                } else {
                    let changed: Vec<_> = data
                        .entries
                        .iter()
                        .filter(|entry| {
                            watch.previous.iter().find(|old| old.id == entry.id) != Some(*entry)
                        })
                        .cloned()
                        .collect();
                    if !changed.is_empty()
                        || !evicted.is_empty()
                        || status != watch.status
                        || data.unknown_records != watch.unknown_records
                        || data.dropped_entries != watch.dropped_entries
                        || was_delayed
                    {
                        // Each batch reserves half the frame for the envelope.
                        // Evictions precede inserts, with <=1000 IDs in total.
                        let mut evictions = Vec::new();
                        let mut size = 0;
                        for id in evicted {
                            let encoded = serde_json::to_vec(&id)?.len() + 1;
                            if !evictions.is_empty() && size + encoded > MAX_RECORD / 2 {
                                self.event(out, "timeline.updated", json!({"identity":identity,"generation":self.generation,"entries":[],"evicted_ids":evictions,"agent_status":status,"unknown_records":data.unknown_records,"dropped_entries":data.dropped_entries,"delayed":false}))?;
                                evictions.clear();
                                size = 0;
                            }
                            evictions.push(id);
                            size += encoded;
                        }
                        let mut batch = Vec::new();
                        for entry in changed {
                            let encoded = serde_json::to_vec(&entry)?.len() + 1;
                            if (!batch.is_empty() || !evictions.is_empty())
                                && size + encoded > MAX_RECORD / 2
                            {
                                self.event(out, "timeline.updated", json!({"identity":identity,"generation":self.generation,"entries":batch,"evicted_ids":evictions,"agent_status":status,"unknown_records":data.unknown_records,"dropped_entries":data.dropped_entries,"delayed":false}))?;
                                batch.clear();
                                evictions.clear();
                                size = 0;
                            }
                            batch.push(entry);
                            size += encoded;
                        }
                        self.event(out, "timeline.updated", json!({"identity":identity,"generation":self.generation,"entries":batch,"evicted_ids":evictions,"agent_status":status,"unknown_records":data.unknown_records,"dropped_entries":data.dropped_entries,"delayed":false}))?;
                    }
                }
                watch.unknown_records = data.unknown_records;
                watch.dropped_entries = data.dropped_entries;
                watch.previous = data.entries.clone();
                if status == "blocked" {
                    if let Some(file) = approval_file {
                        if self.include_approvals {
                            pending = file.try_clone().ok().and_then(|file|
                                self.read_pending(&identity, file, &mut watch.approval_cache))
                                .map(PendingInteraction::Approval);
                        }
                        if pending.is_none() && self.include_questions {
                            pending = self.read_question(&identity, file, &mut watch.question_cache)
                                .map(PendingInteraction::Question);
                        }
                    }
                }
                watch.identity = Some(identity);
                watch.status = status;
                watch.unavailable = None;
                watch.unavailable_agent = None;
                watch.last_daemon_status = None;
                watch.generation = watch.follower.generation;
                if self.include_model_control {
                    self.update_model(out, &mut watch, model_file)?;
                }
            }
        }
        if (self.include_approvals || self.include_questions) && !preserve_pending {
            self.update_pending(out, &mut watch, pending)?;
        }
        self.watch = Some(watch);
        Ok(())
    }
    fn approval_input_stamp(
        &self,
        identity: &Identity,
        file: &std::fs::File,
    ) -> Option<ApprovalInputStamp> {
        let dir = self.spool_dir.as_deref()?;
        let directory = std::fs::symlink_metadata(dir).ok()?;
        if !directory.is_dir() || directory.file_type().is_symlink() {
            return None;
        }
        let path = dir.join(format!("{}-{}.jsonl", identity.agent, identity.native_session_id));
        let spool = std::fs::symlink_metadata(path).ok()?;
        let transcript = file.metadata().ok()?;
        if !spool.is_file() || !transcript.is_file() {
            return None;
        }
        Some(ApprovalInputStamp {
            spool: ApprovalFileStamp::of(&spool),
            transcript: ApprovalFileStamp::of(&transcript),
        })
    }
    // Match only when the source files changed; the visible dialog remains a
    // live read in read_pending. The tail I/O boundary mirrors poll's refresh
    // boundary so tests can verify that cache hits do no tail reading.
    fn matched_request(
        &self,
        identity: &Identity,
        file: std::fs::File,
        cache: &mut Option<ApprovalMatchCache>,
        mut read_tail: impl FnMut(std::fs::File) -> Result<Vec<Value>>,
    ) -> Option<PendingRequest> {
        let stamp = match self.approval_input_stamp(identity, &file) {
            Some(stamp) => stamp,
            None => {
                *cache = None;
                return None;
            }
        };
        if let Some(cached) = cache.as_ref().filter(|c| c.identity == *identity && c.stamp == stamp) {
            return cached.request.clone();
        }
        *cache = None;
        let agent = Agent::parse(&identity.agent)?;
        let mut records = spool::read_session(
            self.spool_dir.as_deref()?, agent, &identity.native_session_id,
        );
        records.retain(|record| {
            record["session_id"] == identity.native_session_id
                && record["payload"]["session_id"] == identity.native_session_id
        });
        if records.is_empty() {
            return None;
        }
        let history = read_tail(file.try_clone().ok()?).ok()?;
        let requests = match agent {
            Agent::Claude => approval_match::match_claude_records(&records, &history),
            Agent::Codex => approval_match::match_codex_records(&records, &history),
        };
        let request = requests.into_iter().rfind(|request| {
            let resolution = match agent {
                Agent::Claude => approval_match::claude_resolution(&request.request_id, &history),
                Agent::Codex => match request.turn_id.as_deref() {
                    Some(turn) => approval_match::codex_resolution(&request.request_id, turn, &history),
                    None => return false,
                },
            };
            resolution == Resolution::Pending
        });
        // Never retain a result assembled while either input was changing.
        if self.approval_input_stamp(identity, &file).as_ref() != Some(&stamp) {
            return None;
        }
        *cache = Some(ApprovalMatchCache {
            identity: identity.clone(), stamp, request: request.clone(),
        });
        request
    }
    fn read_pending(
        &self,
        identity: &Identity,
        file: std::fs::File,
        cache: &mut Option<ApprovalMatchCache>,
    ) -> Option<PendingApproval> {
        let request = self.matched_request(identity, file, cache, crate::follower::read_tail)?;
        let agent = request.agent;
        let command = request.tool_input["command"].as_str()?;
        let screen = self.client.read_visible(&identity.pane_id).ok()?;
        let dialog = approval_screen::parse(agent, &screen, command)?;
        Some(PendingApproval {
            identity: identity.clone(), generation: self.generation, request, dialog,
        })
    }
    fn question_input_stamp(&self, identity: &Identity, file: &std::fs::File) -> Option<QuestionInputStamp> {
        match Agent::parse(&identity.agent)? {
            Agent::Claude => self.approval_input_stamp(identity, file).map(QuestionInputStamp::Claude),
            Agent::Codex => {
                let meta = file.metadata().ok()?;
                meta.is_file().then(|| QuestionInputStamp::Codex(ApprovalFileStamp::of(&meta)))
            }
        }
    }
    fn matched_question(
        &self,
        identity: &Identity,
        file: std::fs::File,
        cache: &mut Option<QuestionMatchCache>,
    ) -> Option<PendingQuestion> {
        let stamp = match self.question_input_stamp(identity, &file) {
            Some(stamp) => stamp,
            None => { *cache = None; return None; }
        };
        if let Some(cached) = cache.as_ref()
            .filter(|c| c.identity == *identity && c.stamp == stamp) {
            cached.request.clone()
        } else {
            *cache = None;
            let history = crate::follower::read_tail(file.try_clone().ok()?).ok()?;
            let requests = match Agent::parse(&identity.agent)? {
                Agent::Claude => {
                    let mut records = spool::read_session(self.spool_dir.as_deref()?, Agent::Claude,
                        &identity.native_session_id);
                    records.retain(|r| r["session_id"] == identity.native_session_id
                        && r["payload"]["session_id"] == identity.native_session_id);
                    question_match::pending_claude_spool_questions(&records, &history)
                },
                Agent::Codex => question_match::pending_codex_questions(&history),
            };
            // A queued banner cannot identify which async call shift+left opens,
            // even when the calls have identical questions. Preserve Plan selection.
            let async_count = requests.iter().filter(|r| r.kind == QuestionKind::CodexAsync).count();
            let request = requests.into_iter().last()
                .filter(|r| r.kind != QuestionKind::CodexAsync || async_count == 1);
            if self.question_input_stamp(identity, &file).as_ref() != Some(&stamp) { return None; }
            *cache = Some(QuestionMatchCache { identity: identity.clone(), stamp, request: request.clone() });
            request
        }
    }
    fn read_question(
        &self,
        identity: &Identity,
        file: std::fs::File,
        cache: &mut Option<QuestionMatchCache>,
    ) -> Option<PendingQuestionEvent> {
        let request = self.matched_question(identity, file, cache)?;
        if request.questions.iter().any(|q| q.secret)
            || (request.kind == QuestionKind::CodexAsync && request.questions.len() != 1) {
            return None;
        }
        let visible = self.client.read_visible(&identity.pane_id).ok()?;
        let screen = question_screen::parse(request.kind, &visible, &request)?;
        if screen.step == QuestionStep::AsyncQueued && screen.queued_count != Some(1) {
            return None;
        }
        let fresh = screen.is_fresh(&request);
        Some(PendingQuestionEvent { identity: identity.clone(), generation: self.generation,
            request, screen, fresh })
    }
    fn update_pending(
        &mut self,
        out: &mut impl Write,
        watch: &mut Watch,
        pending: Option<PendingInteraction>,
    ) -> io::Result<()> {
        // Include the actual event envelope and a possible preceding resolved
        // event's sequence increment when checking the bounded wire frame.
        let pending = pending.filter(|pending| {
            let frame = json!({"event":"interaction.pending","epoch":self.epoch,
                "seq":self.sequence.saturating_add(2),"data":pending.data()});
            serde_json::to_vec(&frame).is_ok_and(|bytes| bytes.len() < MAX_RECORD)
        });
        let old_data = watch.pending.as_ref().map(PendingInteraction::data);
        let new_data = pending.as_ref().map(PendingInteraction::data);
        let same_request = old_data.as_ref().zip(new_data.as_ref()).is_some_and(|(old, new)|
            ["identity", "generation", "request_id", "kind"].iter().all(|key| old[key] == new[key]));
        if !same_request && let Some(old) = watch.pending.take() {
            let old = old.data();
            self.event(out, "interaction.resolved", json!({"identity":old["identity"],
                "generation":old["generation"],"request_id":old["request_id"]}))?;
        }
        if let Some(new) = pending {
            let dialog_changed = match (watch.pending.as_ref(), &new) {
                (Some(PendingInteraction::Approval(old)), PendingInteraction::Approval(new)) =>
                    old.dialog.canonical() != new.dialog.canonical(),
                _ => false,
            };
            if old_data != new_data || dialog_changed {
                self.event(out, "interaction.pending", new.data())?;
            }
            watch.pending = Some(new);
        }
        Ok(())
    }
    fn update_model(
        &mut self,
        out: &mut impl Write,
        watch: &mut Watch,
        file: Option<std::fs::File>,
    ) -> io::Result<()> {
        let Some(file) = file else {
            return Ok(());
        };
        let Some(identity) = watch.identity.clone() else {
            return Ok(());
        };
        let stamp = match file.metadata() {
            Ok(metadata) if metadata.is_file() => {
                use std::os::unix::fs::MetadataExt;
                (metadata.len(), (metadata.mtime(), metadata.mtime_nsec()))
            }
            _ => return Ok(()),
        };
        let live_footer =
            identity.agent == "codex" && matches!(watch.status.as_str(), "idle" | "done");
        if watch.model_stamp == Some(stamp) && !live_footer {
            return Ok(());
        }
        // Claude can skip unchanged transcript scans; Codex idle footers must
        // remain live even when session-only changes leave that stamp unchanged.
        watch.model_stamp = Some(stamp);
        let Some(pane) = watch.source_pane.as_ref() else {
            return Ok(());
        };
        let observed = self.model_scan_raw(&identity.agent, pane, file);
        let observed = ModelOverride::resolve(&mut watch.model_override, observed);
        let Some(observed) = observed else {
            return Ok(());
        };
        if watch.last_model.as_ref() != Some(&observed) {
            watch.last_model = Some(observed.clone());
            let label = watch
                .model_override
                .as_ref()
                .and_then(|value| value.label_for(&Some(observed.clone())));
            // Always a labeled object, so clients never fall back to raw ids.
            let label = label.unwrap_or_else(|| match identity.agent.as_str() {
                "claude" => model_control::claude_display_label(&observed.0),
                _ => self
                    .codex_catalog()
                    .iter()
                    .find(|model| model.slug == observed.0)
                    .map(|model| model.label.clone())
                    .unwrap_or_else(|| model_control::codex_slug_label(&observed.0)),
            });
            let model = json!({"id":observed.0,"label":label});
            self.event(
                out,
                "model.updated",
                json!({"identity":identity,"generation":self.generation,
                    "model":model,"effort":observed.1}),
            )?;
        }
        Ok(())
    }
    /// Fallback pair for a session with no turns yet: Claude's persisted
    /// default (settings.json), Codex's config.toml defaults.
    fn model_scan_fallback(&self, agent: &str) -> Option<(String, Option<String>)> {
        match agent {
            "claude" => {
                // A missing settings.json (a fresh install) is empty settings:
                // Claude still runs its default model.
                let text = self
                    .claude_settings_path
                    .as_ref()
                    .and_then(|path| std::fs::read_to_string(path).ok())
                    .unwrap_or_default();
                let settings = model_control::parse_claude_settings(&text);
                // No configured model means Claude's default; its effort can
                // still be set and must not be lost with the model.
                let model = settings
                    .model
                    .clone()
                    .unwrap_or_else(|| model_control::CLAUDE_DEFAULT_MODEL.to_owned());
                let effort = settings.effort_for(&model).map(str::to_owned);
                Some((model, effort))
            }
            "codex" => {
                let home = self
                    .codex_home
                    .clone()
                    .or_else(crate::codex_config::resolve_codex_dir)?;
                let text = std::fs::read_to_string(home.join("config.toml")).ok()?;
                let (model, effort) = model_control::parse_codex_config(&text);
                model.map(|model| (model, effort))
            }
            _ => None,
        }
    }
    fn model_scan_raw(&self, agent: &str, pane: &Pane, file: std::fs::File) -> ModelPair {
        if agent == "codex"
            && matches!(pane.agent_status.as_str(), "idle" | "done")
            && let Ok(screen) = self.client.read_visible(&pane.pane_id)
            && let Some(pair) = model_control::codex_idle_footer(&screen, &self.codex_catalog())
        {
            return Some(pair);
        }
        let scan = |records: &[Value]| match agent {
            "claude" => model_control::claude_scan(records),
            "codex" => model_control::codex_scan(records),
            _ => None,
        };
        let wide = file
            .try_clone()
            .ok()
            .filter(|file| file.metadata().is_ok_and(|meta| meta.len() > 4 * 1024 * 1024));
        let records = crate::follower::read_tail(file).ok()?;
        // A latest record larger than the usual tail (a huge assistant reply,
        // or a turn_context pushed out by one) needs the follower's full
        // bounded window before falling back to settings.
        scan(&records)
            .or_else(|| {
                let records =
                    crate::follower::read_tail_within(wide?, crate::follower::MAX_SOURCE).ok()?;
                scan(&records)
            })
            .or_else(|| self.model_scan_fallback(agent))
    }
    fn model_scan_current(&mut self, agent: &str, pane: &Pane, file: std::fs::File) -> ModelPair {
        let scanned = self.model_scan_raw(agent, pane, file);
        self.resolve_model_scan(scanned)
    }
    fn resolve_model_scan(&mut self, scanned: ModelPair) -> ModelPair {
        match self.watch.as_mut() {
            Some(watch) => ModelOverride::resolve(&mut watch.model_override, scanned),
            None => scanned,
        }
    }
    fn codex_catalog(&self) -> Vec<model_control::CodexCatalogModel> {
        let Some(home) = self
            .codex_home
            .clone()
            .or_else(crate::codex_config::resolve_codex_dir)
        else {
            return Vec::new();
        };
        std::fs::read_to_string(home.join("models_cache.json"))
            .ok()
            .map(|text| model_control::parse_codex_catalog(&text))
            .unwrap_or_default()
    }
    /// Fresh occupant check shared by both model methods: returns the live
    /// pane and the adapter-verified transcript for the watched identity.
    fn model_pane_file(&self, identity: &Identity) -> Result<(Pane, std::fs::File)> {
        let snapshot = self.client.snapshot().map_err(|_| "stale")?;
        let pane = snapshot
            .panes
            .iter()
            .find(|p| p.pane_id == identity.pane_id)
            .ok_or("stale")?
            .clone();
        if pane.terminal_id != identity.terminal_id
            || pane.agent.as_deref() != Some(identity.agent.as_str())
        {
            return Err("stale");
        }
        let adapter = self
            .adapter_for(&identity.agent)
            .ok_or("unsupported_agent")?;
        let (session, file) = adapter.resolve_and_open(&pane).map_err(|_| "stale")?;
        if session != identity.native_session_id {
            return Err("stale");
        }
        Ok((pane, file))
    }
    fn model_status(&mut self) -> Result<Value> {
        if !self.include_model_control {
            return Err("unsupported_capability");
        }
        let watch = self.watch.as_ref().ok_or("not_watching")?;
        let identity = watch
            .identity
            .clone()
            .filter(|_| watch.unavailable.is_none())
            .ok_or("timeline_unavailable")?;
        let (pane, file) = self.model_pane_file(&identity)?;
        let agent = identity.agent.as_str();
        let current = self.model_scan_current(agent, &pane, file);
        let current_id = current.as_ref().map(|(id, _)| id.as_str());
        let (models, efforts) = match agent {
            "claude" => {
                // Claude's default stays selectable so a session can switch back.
                let default = model_control::CLAUDE_DEFAULT_MODEL;
                let mut models: Vec<Value> = std::iter::once(&(default, ""))
                    .chain(model_control::CLAUDE_MODELS.iter())
                    .map(|(alias, _)| {
                        json!({"id":alias,"label":model_control::claude_display_label(alias)})
                    })
                    .collect();
                // An alias row is current only when the session runs exactly
                // what the alias resolves to; an older or dated id of the same
                // tier (claude-haiku-4-5-20251001) gets its own checked row, so
                // picking the alias is a real switch rather than a no-op.
                let matches_alias = |alias: &str| {
                    current_id.is_some_and(|id| {
                        id == alias || model_control::claude_alias_id(alias).as_deref() == Some(id)
                    })
                };
                let mut marked = false;
                for entry in &mut models {
                    if entry["id"].as_str().is_some_and(matches_alias) {
                        entry["current"] = json!(true);
                        // The transcript's concrete id carries the real version.
                        if let Some(id) = current_id {
                            entry["label"] = json!(model_control::claude_display_label(id));
                        }
                        marked = true;
                    }
                }
                if !marked && let Some(id) = current_id {
                    models.push(json!({"id":id,"label":model_control::claude_display_label(id),"current":true}));
                }
                let efforts = model_control::CLAUDE_EFFORTS
                    .iter()
                    .map(|effort| json!({"id":effort,"label":model_control::effort_label(effort)}))
                    .collect::<Vec<_>>();
                // Every Claude tier offers the same static effort set.
                for entry in &mut models {
                    entry["efforts"] = json!(efforts);
                }
                (models, efforts)
            }
            "codex" => {
                let catalog = self.codex_catalog();
                let fallback = model_control::CODEX_EFFORTS
                    .iter()
                    .map(|effort| json!({"id":effort,"label":model_control::effort_label(effort)}))
                    .collect::<Vec<_>>();
                let mut models: Vec<Value> = catalog
                    .iter()
                    .map(|model| {
                        let efforts: Vec<Value> = model
                            .efforts
                            .iter()
                            .map(|effort| {
                                json!({"id":effort,"label":model_control::effort_label(effort)})
                            })
                            .collect();
                        json!({"id":model.slug,"label":model.label,"efforts":efforts})
                    })
                    .collect();
                let mut marked = false;
                for entry in &mut models {
                    if entry["id"].as_str() == current_id {
                        entry["current"] = json!(true);
                        marked = true;
                    }
                }
                if !marked && let Some(id) = current_id {
                    models.push(
                        json!({"id":id,"label":model_control::codex_slug_label(id),"current":true,"efforts":fallback}),
                    );
                }
                let efforts: Vec<Value> = models
                    .iter()
                    .find(|entry| entry["current"] == true)
                    .and_then(|entry| entry["efforts"].as_array().cloned())
                    .unwrap_or_else(|| fallback.clone());
                (models, efforts)
            }
            _ => return Err("unsupported_agent"),
        };
        let confirmed_label = self
            .watch
            .as_ref()
            .and_then(|watch| watch.model_override.as_ref())
            .and_then(|value| value.label_for(&current));
        let current_label = confirmed_label.or_else(|| {
            current.as_ref().map(|(id, _)| match agent {
                "claude" => model_control::claude_display_label(id),
                _ => self
                    .codex_catalog()
                    .iter()
                    .find(|model| model.slug == *id)
                    .map(|model| model.label.clone())
                    .unwrap_or_else(|| model_control::codex_slug_label(id)),
            })
        });
        Ok(json!({
            "identity":identity, "generation":self.generation, "agent":agent,
            "model": current.as_ref().and_then(|(id, _)| {
                current_label.clone().map(|label| json!({"id":id, "label":label}))
            }),
            "effort": current.as_ref().and_then(|(_, effort)| effort.clone()),
            "models":models, "efforts":efforts,
        }))
    }
    fn model_set(&mut self, params: &Value) -> Result<Value> {
        if !self.include_model_control {
            return Err("unsupported_capability");
        }
        let watch = self.watch.as_ref().ok_or("not_watching")?;
        let identity = watch
            .identity
            .clone()
            .filter(|_| watch.unavailable.is_none())
            .ok_or("timeline_unavailable")?;
        let agent = identity.agent.as_str();
        let target_model = params["model"].as_str();
        let target_effort = params["effort"].as_str();
        model_control::valid_set_target(agent, target_model, target_effort)?;
        if !params["expect"].is_object() {
            return Err("invalid_request");
        }
        let expect_model = params["expect"]["model"].as_str();
        let expect_effort = params["expect"]["effort"].as_str();
        let (pane, file) = self.model_pane_file(&identity)?;
        if !matches!(pane.agent_status.as_str(), "idle" | "done") {
            return Err("agent_busy");
        }
        let scanned_at_set = self.model_scan_raw(agent, &pane, file);
        let current = self.resolve_model_scan(scanned_at_set.clone());
        if current.as_ref().map(|(id, _)| id.as_str()) != expect_model
            || current.as_ref().and_then(|(_, effort)| effort.as_deref()) != expect_effort
        {
            return Err("stale");
        }
        let result = match agent {
            "claude" => self.set_claude_model(&identity, &current, target_model, target_effort)?,
            "codex" => self.set_codex_model(&identity, &current, target_model, target_effort)?,
            _ => return Err("unsupported_agent"),
        };
        // Force the next poll to re-scan and emit `model.updated` for the new
        // pair (the confirmation may never reach the transcript).
        if let Some(watch) = self.watch.as_mut() {
            watch.model_stamp = None;
            let applied = result["applied"]["model"]["id"].as_str().map(|id| {
                (
                    id.to_owned(),
                    result["applied"]["effort"].as_str().map(str::to_owned),
                )
            });
            if result["confirmed"] == true || applied != current {
                watch.model_override = Some(ModelOverride {
                    applied,
                    applied_label: result["applied"]["model"]["label"]
                        .as_str()
                        .map(str::to_owned),
                    scanned_at_set,
                });
            }
        }
        Ok(result)
    }
    /// Decline only a currently visible cache-switch dialog on the same idle occupant.
    fn decline_claude_switch(&self, identity: &Identity) {
        if self.recheck_idle(identity).is_err() {
            return;
        }
        let Ok(screen) = self.client.read_visible(&identity.pane_id) else {
            return;
        };
        if model_control::claude_switch_dialog(&screen).is_some_and(|dialog| dialog.can_decline()) {
            let _ = self.client.send_keys(&identity.pane_id, &["2"]);
        }
    }
    /// Wait for a direct result or accept the matching cache-switch dialog once.
    /// Result lines must follow the submitted command and differ from the baseline.
    fn wait_claude_confirmation(
        &self,
        identity: &Identity,
        baseline: &str,
        command: &str,
        want_effort: Option<&str>,
    ) -> (Option<String>, Option<&'static str>) {
        let pane_id = &identity.pane_id;
        let mut baseline = baseline.to_owned();
        let mut deadline = Instant::now() + Duration::from_secs(4);
        let mut accepted = false;
        let want_model = want_effort
            .is_none()
            .then(|| command.trim_start_matches("/model "));
        loop {
            let screen = self.client.read_visible(pane_id).unwrap_or_default();
            if screen != baseline {
                if let Some(dialog) = model_control::claude_switch_dialog(&screen) {
                    if !dialog.matches(want_model, want_effort) {
                        self.decline_claude_switch(identity);
                        return (None, Some("picker_mismatch"));
                    }
                    if !accepted {
                        if self.recheck_idle(identity).is_err() {
                            return (None, Some("stale"));
                        }
                        // A fresh read must still show exactly the verified dialog.
                        let fresh = self.client.read_visible(pane_id).unwrap_or_default();
                        if model_control::claude_switch_dialog(&fresh).as_ref() == Some(&dialog) {
                            if self.client.send_keys(pane_id, &["1"]).is_err() {
                                return (None, Some("herdr_unavailable"));
                            }
                            accepted = true;
                            baseline = fresh;
                            deadline = Instant::now() + Duration::from_secs(4);
                        }
                    }
                } else if let Some((_, outcome)) = screen.rsplit_once(command) {
                    match model_control::claude_confirmation(outcome) {
                        // A desk-side /model after ours can be the newest line;
                        // accept only a confirmation naming the requested model.
                        Some(ClaudeConfirmation::Model(label)) if want_effort.is_none() => {
                            let wanted = want_model.unwrap_or_default();
                            if !model_control::claude_label_names(&label, wanted) {
                                return (None, Some("stale"));
                            }
                            return (Some(label), None);
                        }
                        Some(ClaudeConfirmation::Effort(level))
                            if want_effort.is_some_and(|wanted| level == wanted) =>
                        {
                            return (Some(level), None);
                        }
                        Some(ClaudeConfirmation::Kept) => return (None, Some("change_declined")),
                        Some(ClaudeConfirmation::Invalid(payload)) => {
                            let expected_invalid = match want_effort {
                                None => payload.contains(command.trim_start_matches("/model ")),
                                Some(wanted) => payload == wanted,
                            };
                            if expected_invalid {
                                return (
                                    None,
                                    Some(if want_effort.is_none() {
                                        "unknown_model"
                                    } else {
                                        "invalid_effort"
                                    }),
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            if Instant::now() >= deadline {
                self.decline_claude_switch(identity);
                return (None, Some("confirm_timeout"));
            }
            thread::sleep(Duration::from_millis(150));
        }
    }
    /// Proves the agent's composer is empty before submitting a prompt through
    /// `agent.prompt`, which would otherwise append to a desk-side draft.
    /// Fails closed: an unreadable or unparsable screen refuses the send.
    fn composer_clear(&self, identity: &Identity, agent: &str) -> Result<()> {
        self.composer_empty(&identity.pane_id, agent)?;
        // Re-verify the occupant last, immediately before the caller's prompt,
        // so a pane replaced after the composer read never receives it.
        self.recheck_idle(identity)
    }
    fn composer_empty(&self, pane_id: &str, agent: &str) -> Result<()> {
        let screen = self
            .client
            .read_visible(pane_id)
            .map_err(|_| "composer_not_empty")?;
        match model_control::composer_state(agent, &screen) {
            model_control::Composer::Empty => Ok(()),
            // Placeholder-shaped text may be the user's draft: accept it only
            // when the styled screen shows it faint, as idle suggestions are.
            model_control::Composer::Placeholder(text) => {
                let styled = self
                    .client
                    .read_visible_ansi(pane_id)
                    .map_err(|_| "composer_not_empty")?;
                if model_control::placeholder_is_faint(agent, &styled, &text) {
                    Ok(())
                } else {
                    Err("composer_not_empty")
                }
            }
            _ => Err("composer_not_empty"),
        }
    }
    /// Occupant and idle/done re-verification before every keystroke batch.
    /// Mid-flow failures map to `stale`, matching `interaction.respond`.
    fn recheck_idle(&self, identity: &Identity) -> Result<()> {
        let (pane, _) = self.model_pane_file(identity)?;
        if !matches!(pane.agent_status.as_str(), "idle" | "done") {
            return Err("stale");
        }
        Ok(())
    }
    fn set_claude_model(
        &self,
        identity: &Identity,
        current: &Option<(String, Option<String>)>,
        target_model: Option<&str>,
        target_effort: Option<&str>,
    ) -> Result<Value> {
        let pane_id = identity.pane_id.as_str();
        let mut applied_model = current.clone().map(|(id, _)| {
            let label = self
                .watch
                .as_ref()
                .and_then(|watch| watch.model_override.as_ref())
                .and_then(|value| value.label_for(current))
                .unwrap_or_else(|| model_control::claude_display_label(&id));
            (id, label)
        });
        let mut applied_effort = current.as_ref().and_then(|(_, effort)| effort.clone());
        let mut error: Option<&str> = None;
        if let Some(target) = target_model
            // A checked alias row (`opus`) is unchanged when the session runs the
            // id it resolves to (`claude-opus-5-5`); sending /model would only
            // rewrite the user's saved default.
            .filter(|target| {
                !current.as_ref().is_some_and(|(id, _)| {
                    id == target || model_control::claude_alias_id(target).as_deref() == Some(id)
                })
            })
        {
            let command = format!("/model {target}");
            self.composer_clear(identity, "claude")?;
            let baseline = self.client.read_visible(pane_id).unwrap_or_default();
            self.client
                .agent_prompt(pane_id, &command)
                .map_err(|_| "herdr_unavailable")?;
            let (label, failure) =
                self.wait_claude_confirmation(identity, &baseline, &command, None);
            match (label, failure) {
                (Some(label), _) => applied_model = Some((target.to_owned(), label)),
                (None, Some(code)) => error = Some(code),
                (None, None) => error = Some("confirm_timeout"),
            }
        }
        if error.is_none()
            && let Some(target) = target_effort.filter(|target| {
                Some(*target) != current.as_ref().and_then(|(_, effort)| effort.as_deref())
            })
        {
            let command = format!("/effort {target}");
            // Re-verify occupant and status, then the composer, before the
            // second command; failures keep the applied model step.
            if let Err(code) = self
                .recheck_idle(identity)
                .and_then(|()| self.composer_clear(identity, "claude"))
            {
                error = Some(if code == "composer_not_empty" {
                    code
                } else {
                    "stale"
                });
            } else {
                let baseline = self.client.read_visible(pane_id).unwrap_or_default();
                self.client
                    .agent_prompt(pane_id, &command)
                    .map_err(|_| "herdr_unavailable")?;
                let (level, failure) =
                    self.wait_claude_confirmation(identity, &baseline, &command, Some(target));
                match (level, failure) {
                    (Some(level), _) => applied_effort = Some(level),
                    (None, Some(code)) => error = Some(code),
                    (None, None) => error = Some("confirm_timeout"),
                }
            }
        }
        Ok(json!({
            "identity":identity, "generation":self.generation,
            "applied":{
                "model": applied_model.map(|(id, label)| json!({"id":id,"label":label})),
                "effort": applied_effort,
            },
            "confirmed": error.is_none(), "error": error,
        }))
    }
    /// Never send Escape into the composer: Esc-Esc opens Codex's rewind UI.
    /// An unchanged render cannot authorize another key after the first Escape.
    fn dismiss_codex_picker(&self, identity: &Identity) {
        let pane_id = identity.pane_id.as_str();
        for _ in 0..3 {
            // Never send Escape into a replacement occupant: the pane id can be
            // reused by another terminal or session that has its own picker open.
            if self.model_pane_file(identity).is_err() {
                return;
            }
            let Ok(screen) = self.client.read_visible(pane_id) else {
                return;
            };
            let before = model_control::parse_codex_screen(&screen);
            if before.is_composer() || before.rows().is_empty() {
                return;
            }
            if self.client.send_keys(pane_id, &["esc"]).is_err() {
                return;
            }
            let deadline = Instant::now() + Duration::from_millis(1500);
            loop {
                thread::sleep(Duration::from_millis(150));
                let Ok(screen) = self.client.read_visible(pane_id) else {
                    return;
                };
                let after = model_control::parse_codex_screen(&screen);
                if after.is_composer() || after.rows().is_empty() {
                    return;
                }
                if std::mem::discriminant(&after) != std::mem::discriminant(&before) {
                    break;
                }
                if Instant::now() >= deadline {
                    return;
                }
            }
        }
    }
    fn wait_codex_kind(
        &self,
        pane_id: &str,
        matches: impl Fn(&PickerScreen) -> bool,
    ) -> Option<PickerScreen> {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let screen = self.client.read_visible(pane_id).unwrap_or_default();
            let parsed = model_control::parse_codex_screen(&screen);
            if matches(&parsed) {
                return Some(parsed);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(150));
        }
    }
    fn wait_codex_toast(
        &self,
        pane_id: &str,
        baseline: &str,
        slug: &str,
        effort: Option<&str>,
    ) -> Option<(String, String)> {
        let deadline = Instant::now() + Duration::from_secs(4);
        loop {
            let screen = self.client.read_visible(pane_id).unwrap_or_default();
            if let Some(applied) =
                model_control::codex_fresh_change(&screen, baseline, slug, effort)
            {
                return Some(applied);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(150));
        }
    }
    /// Re-reads the picker and confirms the expected screen with its cursor on
    /// `target` (or on any row when `target` is None).
    fn verify_cursor(&self, pane_id: &str, expected: &PickerScreen, target: Option<usize>) -> bool {
        let deadline = Instant::now() + Duration::from_millis(1500);
        loop {
            let matches = || -> bool {
                let Ok(screen) = self.client.read_visible(pane_id) else {
                    return false;
                };
                let parsed = model_control::parse_codex_screen(&screen);
                match (expected, &parsed) {
                    (PickerScreen::ModelList(_), PickerScreen::ModelList(_))
                    | (PickerScreen::Advanced(_), PickerScreen::Advanced(_)) => {}
                    (
                        PickerScreen::EffortList {
                            model_label: expected,
                            ..
                        },
                        PickerScreen::EffortList {
                            model_label: actual,
                            ..
                        },
                    ) if model_control::picker_label(expected)
                        .eq_ignore_ascii_case(model_control::picker_label(actual)) => {}
                    _ => return false,
                }
                if !expected.rows().iter().zip(parsed.rows()).all(|(a, b)| {
                    a.number == b.number
                        && model_control::picker_label(&a.label)
                            .eq_ignore_ascii_case(model_control::picker_label(&b.label))
                }) || expected.rows().len() != parsed.rows().len()
                {
                    return false;
                }
                match target {
                    Some(target) => parsed.cursor() == Some(target),
                    None => parsed.cursor().is_some(),
                }
            };
            if matches() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(150));
        }
    }

    fn set_codex_model(
        &self,
        identity: &Identity,
        current: &Option<(String, Option<String>)>,
        target_model: Option<&str>,
        target_effort: Option<&str>,
    ) -> Result<Value> {
        let pane_id = identity.pane_id.as_str();
        let current_slug = current.as_ref().map(|(id, _)| id.as_str());
        let target_slug = target_model.or(current_slug).ok_or("invalid_request")?;
        let catalog = self.codex_catalog();
        let label_for = |slug: &str| {
            catalog
                .iter()
                .find(|model| model.slug == slug)
                .map(|model| model.label.clone())
                .unwrap_or_else(|| model_control::codex_slug_label(slug))
        };
        // Reselecting the current model, with no effort or the current one, is
        // a no-op: walking the picker would apply its default row (replacing an
        // effort it can't show, like minimal), and an identical toast can't
        // confirm anything.
        let current_effort = current.as_ref().and_then(|(_, effort)| effort.as_deref());
        if Some(target_slug) == current_slug
            && target_effort.is_none_or(|effort| Some(effort) == current_effort)
        {
            return Ok(json!({
                "identity":identity, "generation":self.generation,
                "applied":{
                    "model":{"id":target_slug,"label":label_for(target_slug)},
                    "effort": current.as_ref().and_then(|(_, effort)| effort.clone()),
                },
                "confirmed": true, "error": null,
            }));
        }
        let target_label = label_for(target_slug);
        let mismatch = || -> &'static str {
            self.dismiss_codex_picker(identity);
            "picker_mismatch"
        };
        // Mid-flow occupant/status failures dismiss the open picker first.
        let stale = || -> &'static str {
            self.dismiss_codex_picker(identity);
            "stale"
        };
        let send = |keys: &[&str]| -> Result<()> {
            self.recheck_idle(identity).map_err(|_| stale())?;
            self.client
                .send_keys(pane_id, keys)
                .map_err(|_| "herdr_unavailable")
        };
        // Re-verify occupant, arrows to `target`, verify the landed cursor,
        // and re-verify again — every keystroke batch is bracketed.
        let walk = |screen: &PickerScreen, target: usize| -> Result<()> {
            self.recheck_idle(identity).map_err(|_| stale())?;
            for arrow in screen.arrow_plan(target).ok_or_else(mismatch)? {
                send(&[arrow])?;
            }
            if !self.verify_cursor(pane_id, screen, Some(target)) {
                return Err(mismatch());
            }
            self.recheck_idle(identity).map_err(|_| stale())?;
            Ok(())
        };

        self.composer_clear(identity, "codex")?;
        self.client
            .agent_prompt(pane_id, "/model")
            .map_err(|_| "herdr_unavailable")?;
        let model_list = self
            .wait_codex_kind(
                pane_id,
                |screen| matches!(screen, PickerScreen::ModelList(rows) if !rows.is_empty()),
            )
            .ok_or_else(mismatch)?;
        let PickerScreen::ModelList(_) = &model_list else {
            return Err(mismatch());
        };
        // Enter on the model row only advances to the effort step; it never
        // persists anything (verified against the isolated picker).
        let target_row = model_list
            .row_for_label(&target_label)
            .ok_or_else(mismatch)?;
        walk(&model_list, target_row)?;
        send(&["enter"])?;

        let effort_screen = self
            .wait_codex_kind(pane_id, |screen| {
                matches!(screen, PickerScreen::EffortList { model_label, rows }
                    if !rows.is_empty() && model_control::picker_label(model_label).eq_ignore_ascii_case(model_control::picker_label(&target_label)))
            })
            .ok_or_else(mismatch)?;
        let PickerScreen::EffortList { rows, .. } = &effort_screen else {
            return Err(mismatch());
        };

        // Re-picking the current effort still applies cleanly, but only when
        // the target model offers it; otherwise the target's default cursor
        // position applies (a model switch must not fail on a carried effort).
        let carried = current
            .as_ref()
            .and_then(|(_, effort)| effort.clone())
            .filter(|effort| match catalog.iter().find(|model| model.slug == target_slug) {
                Some(model) => model.efforts.iter().any(|offered| offered == effort),
                None => model_control::codex_effort_row_label(effort).is_some_and(|label| {
                    rows.iter().any(|row| {
                        model_control::picker_label(&row.label).starts_with(label)
                    })
                }),
            });
        if let Some(effort) = target_effort.map(str::to_owned).or(carried) {
            let row_label =
                model_control::codex_effort_row_label(&effort).ok_or_else(mismatch)?;
            if row_label.starts_with("More reasoning") {
                let index = rows
                    .iter()
                    .position(|row| {
                        model_control::picker_label(&row.label).starts_with("More reasoning")
                    })
                    .ok_or_else(mismatch)?;
                walk(&effort_screen, index)?;
                send(&["enter"])?;
                let advanced = self
                    .wait_codex_kind(
                        pane_id,
                        |screen| matches!(screen, PickerScreen::Advanced(rows) if !rows.is_empty()),
                    )
                    .ok_or_else(mismatch)?;
                let PickerScreen::Advanced(_) = &advanced else {
                    return Err(mismatch());
                };
                let label = model_control::effort_label(&effort);
                let index = advanced.row_for_label(label).ok_or_else(mismatch)?;
                walk(&advanced, index)?;
            } else {
                let index = effort_screen
                    .row_for_label(row_label)
                    .ok_or_else(mismatch)?;
                walk(&effort_screen, index)?;
            }
        } else {
            // No desired effort: any selected default row applies.
            if !self.verify_cursor(pane_id, &effort_screen, None) {
                return Err(mismatch());
            }
            self.recheck_idle(identity).map_err(|_| stale())?;
        }
        // `s` applies for this session only; digits or enter would persist a
        // new global default in config.toml.
        let baseline = self.client.read_visible(pane_id).map_err(|_| mismatch())?;
        send(&["s"])?;

        let Some((applied_slug, applied_effort)) =
            self.wait_codex_toast(pane_id, &baseline, target_slug, target_effort)
        else {
            self.dismiss_codex_picker(identity);
            return Err("confirm_timeout");
        };
        let applied_label = if applied_slug == target_slug {
            target_label
        } else {
            model_control::codex_slug_label(&applied_slug)
        };
        Ok(json!({
            "identity":identity, "generation":self.generation,
            "applied":{
                "model":{"id":applied_slug,"label":applied_label},
                "effort":applied_effort,
            },
            "confirmed":true, "error":null,
        }))
    }
    fn respond_interaction(&self, params: &Value) -> InteractionResult<Value> {
        let question_response = params.get("answers").is_some() || params.get("skip").is_some();
        if (!self.include_approvals && !self.include_questions) || (question_response && !self.include_questions) {
            return Err("unsupported_capability".into());
        }
        let watch = self.watch.as_ref().ok_or("not_watching")?;
        let identity = watch.identity.as_ref().filter(|_| watch.unavailable.is_none())
            .ok_or("timeline_unavailable")?;
        match watch.pending.as_ref().ok_or("stale")? {
            PendingInteraction::Approval(pending) => {
                if !self.include_approvals { return Err("unsupported_capability".into()); }
                if params["request_id"].as_str() != Some(&pending.request.request_id)
                    || pending.identity != *identity || pending.generation != self.generation {
                    return Err("stale".into());
                }
                if question_response { return Err("mismatch".into()); }
                self.respond_approval(params, watch, identity, pending).map_err(Into::into)
            }
            PendingInteraction::Question(pending) => {
                if !self.include_questions { return Err("unsupported_capability".into()); }
                if params["request_id"].as_str() != Some(&pending.request.request_id)
                    || pending.identity != *identity || pending.generation != self.generation {
                    return Err("stale".into());
                }
                self.respond_question(params, watch, identity, pending)
            }
        }
    }
    fn respond_approval(&self, params: &Value, watch: &Watch, identity: &Identity, pending: &PendingApproval) -> Result<Value> {
        let command = pending.request.tool_input["command"].as_str().ok_or("stale")?;
        let screen = self.client.read_visible(&identity.pane_id).map_err(|_| "stale")?;
        let current = approval_screen::parse(pending.request.agent, &screen, command).ok_or("stale")?;
        if !pending.dialog.same_dialog(&current) {
            return Err("stale");
        }
        let key = pending.response_key(params)?;

        // Reopen through the adapter using the last verified pane reference.
        // A response always bypasses the polling cache and rechecks both files.
        let source = watch.source_pane.as_ref().ok_or("stale")?;
        let adapter = self.adapter_for(&identity.agent).ok_or("stale")?;
        let (session, file) = adapter.resolve_and_open(source).map_err(|_| "stale")?;
        if session != identity.native_session_id {
            return Err("stale");
        }
        let newest = self.matched_request(identity, file, &mut None, crate::follower::read_tail)
            .ok_or("stale")?;
        if newest.request_id != pending.request.request_id
            || newest.tool_name != pending.request.tool_name
            || newest.tool_input != pending.request.tool_input
            || newest.turn_id != pending.request.turn_id
        {
            return Err("stale");
        }

        // Verify the live occupant as well as blocked status immediately before
        // sending. The adapter validates the fresh session reference and cwd.
        let snapshot = self.client.snapshot().map_err(|_| "stale")?;
        let pane = snapshot.panes.iter().find(|p| p.pane_id == identity.pane_id)
            .ok_or("stale")?;
        if pane.agent_status != "blocked" || pane.terminal_id != identity.terminal_id
            || pane.agent.as_deref() != Some(&identity.agent)
        {
            return Err("stale");
        }
        let (session, _) = adapter.resolve_and_open(pane).map_err(|_| "stale")?;
        if session != identity.native_session_id {
            return Err("stale");
        }
        self.client.send_keys(&identity.pane_id, &[key.as_str()])?;

        let deadline = Instant::now() + Duration::from_secs(2);
        let mut confirmed = false;
        for _ in 0..20 {
            let Ok(screen) = self.client.read_visible(&identity.pane_id) else { break };
            let same = approval_screen::parse(pending.request.agent, &screen, command)
                .is_some_and(|dialog| pending.dialog.same_dialog(&dialog));
            if !same {
                confirmed = true;
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            thread::sleep(Duration::from_millis(100).min(remaining));
        }
        Ok(json!({"request_id":pending.request.request_id,"sent":key,"confirmed":confirmed}))
    }
    fn question_pane_file(&self, identity: &Identity) -> Result<std::fs::File> {
        let snapshot = self.client.snapshot().map_err(|_| "stale")?;
        let pane = snapshot.panes.iter().find(|p| p.pane_id == identity.pane_id).ok_or("stale")?;
        if pane.agent_status != "blocked" || pane.terminal_id != identity.terminal_id
            || pane.agent.as_deref() != Some(&identity.agent) { return Err("stale"); }
        let adapter = self.adapter_for(&identity.agent).ok_or("stale")?;
        let (session, file) = adapter.resolve_and_open(pane).map_err(|_| "stale")?;
        if session != identity.native_session_id { return Err("stale"); }
        Ok(file)
    }
    fn respond_question(&self, params: &Value, watch: &Watch, identity: &Identity, pending: &PendingQuestionEvent) -> InteractionResult<Value> {
        if !pending.fresh { return Err("in_progress".into()); }
        // Reopen the last verified reference, then independently validate the fresh
        // occupant. Neither the transcript match nor screen may come from a cache.
        let source = watch.source_pane.as_ref().ok_or("stale")?;
        let adapter = self.adapter_for(&identity.agent).ok_or("stale")?;
        let (session, _) = adapter.resolve_and_open(source).map_err(|_| "stale")?;
        if session != identity.native_session_id { return Err("stale".into()); }
        let file = self.question_pane_file(identity)?;
        let newest = self.matched_question(identity, file, &mut None).ok_or("stale")?;
        if newest != pending.request { return Err("stale".into()); }
        let visible = self.client.read_visible(&identity.pane_id).map_err(|_| "stale")?;
        let current = question_screen::parse(newest.kind, &visible, &newest).ok_or("stale")?;
        if current.step == QuestionStep::AsyncQueued && current.queued_count != Some(1) {
            return Err("stale".into());
        }
        if !current.is_fresh(&newest) { return Err("in_progress".into()); }
        let steps = question_keys::plan(newest.kind, &newest, &current, params)?;
        let confirmed = self.execute_question(identity, &newest, current, &steps)?;
        Ok(json!({"request_id":newest.request_id,"confirmed":confirmed}))
    }
    fn execute_question(&self, identity: &Identity, pending: &PendingQuestion, mut current: QuestionScreen, steps: &[Step]) -> InteractionResult<bool> {
        let mut steps_sent = 0;
        for (index, step) in steps.iter().enumerate() {
            if index > 0 {
                let occupant = self.question_pane_file(identity);
                // A guard read immediately before each later input catches desk
                // edits and a last picker that disappeared after the digit check.
                let visible = self.client.read_visible_with_timeout(&identity.pane_id, Duration::from_secs(1))
                    .map_err(|_| InteractionError::stale_after(steps_sent))?;
                let parsed = question_screen::parse(pending.kind, &visible, pending);
                if step.only_if.is_some() && parsed.is_none() { return Ok(true); }
                let parsed = parsed.ok_or_else(|| InteractionError::stale_after(steps_sent))?;
                // A remaining dialog must still belong to this blocked occupant.
                // Disappearance needs no further input, even if the pane is idle.
                occupant.map_err(|_| InteractionError::stale_after(steps_sent))?;
                if !steps[index-1].expected.matches(Some(&parsed))
                    || step.only_if.as_ref().is_some_and(|guard| !guard.matches(&parsed)) {
                    return Err(InteractionError::stale_after(steps_sent));
                }
                current = parsed;
            }
            // Count dispatched input attempts conservatively: a transport failure
            // may happen after Herdr received the input. Never retry that input.
            steps_sent += 1;
            let sent = match &step.input {
                Input::Keys(keys) => self.client.send_keys(&identity.pane_id, &keys.iter().map(String::as_str).collect::<Vec<_>>()),
                Input::Text(text) => self.client.send_text(&identity.pane_id, text),
            };
            sent.map_err(|_| InteractionError::stale_after(steps_sent))?;
            let deadline = Instant::now() + Duration::from_secs(1);
            let final_input = matches!(step.expected, ExpectedScreen::Gone);
            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    if final_input { return Ok(false); }
                    return Err(InteractionError::stale_after(steps_sent));
                }
                let visible = match self.client.read_visible_with_timeout(&identity.pane_id, remaining) {
                    Ok(visible) => visible,
                    Err(_) if final_input => return Ok(false),
                    Err(_) => return Err(InteractionError::stale_after(steps_sent)),
                };
                let parsed = question_screen::parse(pending.kind, &visible, pending);
                if step.expected.matches(parsed.as_ref()) {
                    match parsed {
                        Some(parsed) => { current = parsed; break; }
                        None => return Ok(true),
                    }
                }
                // On final submission an unchanged dialog means unconfirmed;
                // any different parsed dialog is a state mismatch, not success.
                if final_input && parsed.as_ref().is_none_or(|s| s.fingerprint != current.fingerprint) {
                    return Err(InteractionError::stale_after(steps_sent));
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if !remaining.is_zero() { thread::sleep(Duration::from_millis(100).min(remaining)); }
            }
        }
        Ok(false)
    }
    pub fn request(&mut self, bytes: &[u8], out: &mut impl Write) -> io::Result<()> {
        self.request_with_refresh(bytes, out, |follower, file, session| {
            follower.refresh_file(file, session)
        })
    }
    fn request_with_refresh(
        &mut self,
        bytes: &[u8],
        out: &mut impl Write,
        refresh: impl FnMut(&mut Follower, std::fs::File, &str) -> Result<bool>,
    ) -> io::Result<()> {
        let request: Value = match serde_json::from_slice(bytes) {
            Ok(value) => value,
            Err(_) => return Self::error(out, Value::Null, "invalid_json"),
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        if !id.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 128) {
            return Self::error(out, Value::Null, "invalid_request_id");
        }
        let params = &request["params"];
        let method = request["method"].as_str().unwrap_or("");
        if method == "initialize" {
            self.initialized = false;
            self.include_content = params["record_content"] == true;
            self.include_approvals = params["approvals"] == true;
            self.include_questions = params["questions"] == true;
            self.include_model_control = params["model_control"] == true;
            self.watch = None;
            if params["protocol"] != 1 {
                return Self::error(out, id, "protocol_mismatch");
            }
            if let Err(code) = self.client.snapshot() {
                return Self::error(out, id, code);
            }
            self.initialized = true;
            let claude_available = self.adapter_for("claude").is_some_and(|a| a.is_available());
            let codex_available = self.adapter_for("codex").is_some_and(|a| a.is_available());
            return Self::write(
                out,
                &json!({"id":id,"result":{"protocol":1,"bridge_version":env!("CARGO_PKG_VERSION"),"epoch":self.epoch,"capabilities":{"claude_timeline":claude_available,"record_content":true,"codex_timeline":codex_available,"codex_daemon_config":true,"codex_daemon_stop":true,"git":false,"prompts":false,"approvals":true,"questions":true,"model_control":true,"routed_hosts":false}}}),
            );
        }
        if !self.initialized {
            return Self::error(out, id, "not_initialized");
        }
        match method {
            "interaction.respond" => match self.respond_interaction(params) {
                Ok(result) => {
                    Self::write(out, &json!({"id":id,"result":result}))?;
                    self.poll(out)
                }
                Err(error) => Self::write(out, &json!({"id":id,"error":error.data()})),
            },
            "model.status" => {
                self.poll_with_refresh(out, refresh)?;
                match self.model_status() {
                    Ok(result) => Self::write(out, &json!({"id":id,"result":result})),
                    Err(code) => Self::error(out, id, code),
                }
            }
            "model.set" => {
                self.poll_with_refresh(out, refresh)?;
                match self.model_set(params) {
                    Ok(result) => {
                        Self::write(out, &json!({"id":id,"result":result}))?;
                        self.poll(out)
                    }
                    Err(code) => Self::write(out, &json!({"id":id,"error":{"code":code}})),
                }
            }
            "watch" => {
                let Some(pane) = params["pane_id"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 256)
                else {
                    return Self::error(out, id, "invalid_pane_id");
                };
                self.watch = Some(Watch {
                    pane_id: pane.into(),
                    identity: None,
                    follower: if self.include_content {
                        Follower::with_content()
                    } else {
                        Follower::default()
                    },
                    generation: 0,
                    previous: Vec::new(),
                    unavailable: None,
                    status: String::new(),
                    unknown_records: 0,
                    dropped_entries: 0,
                    source_changing_count: 0,
                    delayed: false,
                    agent: None,
                    unavailable_agent: None,
                    last_daemon_status: None,
                    pending: None,
                    approval_cache: None,
                    question_cache: None,
                    source_pane: None,
                    model_stamp: None,
                    last_model: None,
                    model_override: None,
                });
                Self::write(out, &json!({"id":id,"result":{"watching":pane}}))?;
                self.poll(out)
            }
            "unwatch" => {
                self.watch = None;
                Self::write(out, &json!({"id":id,"result":{"watching":null}}))
            }
            "content" => {
                if !self.include_content {
                    return Self::error(out, id, "unsupported_capability");
                }
                self.poll_with_refresh(out, refresh)?;
                let Some(watch) = &self.watch else {
                    return Self::error(out, id, "not_watching");
                };
                if watch.unavailable.is_some() || watch.identity.is_none() || watch.delayed {
                    return Self::error(out, id, "timeline_unavailable");
                }
                if params["generation"].as_u64() != Some(self.generation) {
                    return Self::error(out, id, "stale_cursor");
                }
                let Some(reference) = params["content_ref"].as_str().filter(|s| s.len() <= 128)
                else {
                    return Self::error(out, id, "content_unavailable");
                };
                let Some(offset) = params["offset"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                else {
                    return Self::error(out, id, "invalid_content_offset");
                };
                match watch.follower.read_content(reference, offset) {
                    Ok((bytes, total)) => {
                        let digits = b"0123456789abcdef";
                        let mut hex = String::with_capacity(bytes.len() * 2);
                        for byte in &bytes {
                            hex.push(digits[(byte >> 4) as usize] as char);
                            hex.push(digits[(byte & 15) as usize] as char);
                        }
                        Self::write(
                            out,
                            &json!({"id":id,"result":{"identity":watch.identity,"generation":self.generation,
                            "content_ref":reference,"offset":offset,"next_offset":offset + bytes.len(),"total":total,"hex":hex}}),
                        )
                    }
                    Err(code) => Self::error(out, id, code),
                }
            }
            "history" => {
                self.poll_with_refresh(out, refresh)?;
                let Some(watch) = &self.watch else {
                    return Self::error(out, id, "not_watching");
                };
                if watch.unavailable.is_some() || watch.identity.is_none() {
                    return Self::error(out, id, "timeline_unavailable");
                }
                if params["generation"].as_u64() != Some(self.generation) {
                    return Self::error(out, id, "stale_cursor");
                }
                let entries = &watch.follower.transcript.entries;
                let Some(before) = params["before"]
                    .as_str()
                    .and_then(|id| entries.iter().position(|e| e.id == id))
                else {
                    return Self::error(out, id, "stale_cursor");
                };
                let mut result = page(entries, before);
                result["identity"] = json!(watch.identity);
                result["generation"] = json!(self.generation);
                result["dropped_entries"] = json!(watch.follower.transcript.dropped_entries);
                Self::write(out, &json!({"id":id,"result":result}))
            }
            _ => Self::error(out, id, "unknown_method"),
        }
    }
}
pub fn serve(mut service: Service, interval: Duration) -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel::<Result<Vec<u8>>>(16);
    thread::spawn(move || {
        let mut input = BufReader::new(io::stdin());
        loop {
            let mut bytes = Vec::new();
            match (&mut input)
                .take((MAX_RECORD + 1) as u64)
                .read_until(b'\n', &mut bytes)
            {
                Ok(0) => break,
                Ok(_) if bytes.len() > MAX_RECORD => {
                    let _ = sender.send(Err("input_frame_too_large"));
                    break;
                }
                Ok(_) if bytes.last() != Some(&b'\n') => {
                    let _ = sender.send(Err("incomplete_input_frame"));
                    break;
                }
                Ok(_) => {
                    if sender.send(Ok(bytes)).is_err() {
                        break;
                    }
                }
                Err(_) => {
                    let _ = sender.send(Err("input_unavailable"));
                    break;
                }
            }
        }
    });
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut deadline = Instant::now() + interval;
    loop {
        match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Ok(bytes)) => service.request(&bytes, &mut out)?,
            Ok(Err(code)) => {
                Service::error(&mut out, Value::Null, code)?;
                return Err(io::Error::other(code));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
        if Instant::now() >= deadline {
            service.poll(&mut out)?;
            deadline = Instant::now() + interval;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::net::UnixListener};
    const SESSION: &str = "16246b9d-4c3d-54fc-b6d3-2a765d418491";
    const OTHER: &str = "11111111-1111-4111-8111-111111111111";

    #[test]
    fn transcript_override_keeps_daemon_home_and_home_override_invalidates_cache() {
        let root = std::env::temp_dir().join(format!(
            "service-home-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let home = root.join("home");
        let transcripts = root.join("other-volume/transcripts");
        fs::create_dir_all(home.join("app-server-daemon")).unwrap();
        fs::create_dir_all(&transcripts).unwrap();
        // A malformed home pid file gives Unknown; the unrelated transcript
        // parent has no pid file and would incorrectly give NotRunning.
        fs::write(home.join("app-server-daemon/daemon.pid"), "invalid").unwrap();
        let mut service = Service::with_resolvers(root.join("unused.sock"), None, None)
            .with_codex_home(root.join("empty-home"));
        assert_eq!(service.cached_codex_daemon_status(), "not_running");
        service = service.with_codex_home(home).with_codex_root(transcripts);
        let status = service.cached_codex_daemon_status();
        fs::remove_dir_all(root).unwrap();
        assert_eq!(status, "unknown");
    }

    // Return snapshots synchronously for each explicit poll. The injected
    // refresh boundary makes source_changing independent of timing/file races.
    #[derive(Default)]
    struct HerdrState {
        ansi_screen: Option<String>,
        // (n, terminal): from the n-th plain pane.read on, snapshots report a
        // replacement terminal (the pane was swapped right after that read).
        terminal_after_reads: Option<(usize, String)>,
        agent: Option<String>,
        status: Option<String>,
        screen: String,
        reads: usize,
        read_error: bool,
        read_error_after_send: bool,
        keys: Vec<Value>,
        screens_after_send: std::collections::VecDeque<String>,
        allow_question_input: bool,
        inputs: Vec<Value>,
        // One render sequence per input; extra guard reads retain the last render.
        screens_after_input: std::collections::VecDeque<std::collections::VecDeque<String>>,
        active_input_screens: std::collections::VecDeque<String>,
        screens_before_input: std::collections::VecDeque<String>,
        terminal_after_input: Option<String>,
        status_after_inputs: Option<(usize, String)>,
        hide_pane: bool,
        terminal_id: Option<String>,
        session_override: Option<String>,
    }
    type SharedHerdr = std::sync::Arc<std::sync::Mutex<HerdrState>>;
    fn fixture() -> (Service, PathBuf, std::sync::Arc<std::sync::Mutex<String>>) {
        let (service, root, session, _) = fixture_with_herdr();
        (service, root, session)
    }
    fn fixture_with_herdr() -> (Service, PathBuf, std::sync::Arc<std::sync::Mutex<String>>, SharedHerdr) {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "service-test-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        fs::create_dir_all(root.join("projects/one")).unwrap();
        for session in [SESSION, OTHER] {
            let text = (0..107).map(|i| format!("{}\n", json!({"type":"user","uuid":format!("m{i}"),"sessionId":session,"message":{"role":"user","content":format!("text{i}")}}))).collect::<String>();
            fs::write(root.join(format!("projects/one/{session}.jsonl")), text).unwrap();
        }
        let socket = root.join("h.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let session = std::sync::Arc::new(std::sync::Mutex::new(SESSION.to_owned()));
        let session_for_server = session.clone();
        let herdr = SharedHerdr::default();
        let server_state = herdr.clone();
        // Cleanup wakes this blocking listener with an empty request.
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut line = String::new();
                if BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .unwrap()
                    == 0
                {
                    break;
                }
                let request: Value = serde_json::from_str(&line).unwrap();
                let mut state = server_state.lock().unwrap();
                let result = match request["method"].as_str().unwrap() {
                    "pane.read" if request["params"]["format"] == "ansi" => {
                        assert_eq!(request["params"], json!({"pane_id":"w1:p1","source":"visible","format":"ansi"}));
                        // Real agents draw idle placeholders faint; a test sets
                        // `ansi_screen` to model typed, unstyled text instead.
                        let text = state.ansi_screen.clone().unwrap_or_else(|| {
                            state.screen.lines().map(|line| {
                                for marker in ["❯ ", "› "] {
                                    if let Some(rest) = line.trim_start().strip_prefix(marker) {
                                        return format!("{marker}\u{1b}[2m{rest}\u{1b}[0m");
                                    }
                                }
                                line.to_owned()
                            }).collect::<Vec<_>>().join("\n")
                        });
                        json!({"read":{"text":text}})
                    }
                    "pane.read" => {
                        assert_eq!(request["params"], json!({"pane_id":"w1:p1","source":"visible"}));
                        state.reads += 1;
                        if let Some((count, terminal)) = state.terminal_after_reads.clone()
                            && state.reads >= count { state.terminal_id = Some(terminal); }
                        if state.allow_question_input {
                            if let Some(screen) = state.screens_before_input.pop_front()
                                .or_else(|| state.active_input_screens.pop_front()) {
                                state.screen = screen;
                            }
                        } else if !state.keys.is_empty() {
                            if let Some(screen) = state.screens_after_send.pop_front() {
                                state.screen = screen;
                            }
                        }
                        json!({"read":{"text":state.screen}})
                    }
                    "pane.send_keys" => {
                        let keys = request["params"]["keys"].as_array().unwrap();
                        if !state.allow_question_input {
                            assert_eq!(keys.len(), 1);
                            assert!(keys.iter().all(|key| !["enter", "\n", "\r"].contains(&key.as_str().unwrap())));
                        }
                        state.keys.push(request["params"].clone());
                        state.inputs.push(json!({"method":"pane.send_keys","params":request["params"]}));
                        state.active_input_screens = state.screens_after_input.pop_front().unwrap_or_default();
                        json!({})
                    }
                    "pane.send_text" => {
                        assert!(state.allow_question_input);
                        assert_eq!(request["params"]["pane_id"], "w1:p1");
                        assert!(request["params"]["text"].is_string());
                        assert_eq!(request["params"].as_object().unwrap().len(), 2);
                        state.inputs.push(json!({"method":"pane.send_text","params":request["params"]}));
                        state.active_input_screens = state.screens_after_input.pop_front().unwrap_or_default();
                        json!({})
                    }
                    "agent.prompt" => {
                        assert!(state.allow_question_input);
                        assert_eq!(request["params"]["target"], "w1:p1");
                        assert!(request["params"]["text"].is_string());
                        assert_eq!(request["params"].as_object().unwrap().len(), 2);
                        state.inputs.push(json!({"method":"agent.prompt","params":request["params"]}));
                        state.active_input_screens = state.screens_after_input.pop_front().unwrap_or_default();
                        json!({})
                    }
                    "session.snapshot" => {
                        if let Some((count, status)) = state.status_after_inputs.clone()
                            && state.inputs.len() >= count { state.status = Some(status); }
                        if !state.inputs.is_empty() && let Some(terminal) = state.terminal_after_input.clone() {
                            state.terminal_id = Some(terminal);
                        }
                        let agent = state.agent.as_deref().unwrap_or("claude");
                        let mut snapshot = json!({"protocol":22,"version":"0.9.3","panes":[{"pane_id":"w1:p1","terminal_id":state.terminal_id.as_deref().unwrap_or("terminal-one"),"workspace_id":"w1","tab_id":"w1:t1","agent":agent,"agent_status":state.status.as_deref().unwrap_or("idle"),"foreground_cwd":"/fixture/work","revision":1,"agent_session":{"agent":agent,"source":format!("herdr:{agent}"),"kind":"id","value":state.session_override.clone().unwrap_or_else(|| session_for_server.lock().unwrap().clone())}}]});
                        if state.hide_pane {
                            snapshot["panes"] = json!([]);
                        }
                        json!({"type":"session_snapshot","snapshot":snapshot})
                    }
                    method => panic!("unexpected method: {method}"),
                };
                let frame = if request["method"] == "pane.read"
                    && (state.read_error || state.read_error_after_send && !state.keys.is_empty())
                {
                    json!({"id":request["id"],"error":{"code":"pane_not_found"}})
                } else {
                    json!({"id":request["id"],"result":result})
                };
                writeln!(stream, "{frame}").unwrap();
            }
        });
        let mut service = Service::new(socket, root.join("projects"));
        service.initialized = true;
        service
            .request(
                br#"{"id":"w","method":"watch","params":{"pane_id":"w1:p1"}}"#,
                &mut Vec::new(),
            )
            .unwrap();
        (service, root, session, herdr)
    }
    include!("service_approval_tests.rs");
    include!("service_question_tests.rs");
    include!("service_question_response_tests.rs");
    include!("service_response_tests.rs");
    include!("service_model_tests.rs");
    fn cleanup(service: &Service, root: PathBuf) {
        // Wake and terminate the fixture thread without a valid request.
        drop(std::os::unix::net::UnixStream::connect(&service.client.socket).unwrap());
        fs::remove_dir_all(root).unwrap();
    }
    fn frames(out: Vec<u8>) -> Vec<Value> {
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    #[test]
    fn changing_same_source_keeps_verified_history_and_generation() {
        let (mut service, root, _) = fixture();
        let generation = service.generation;
        let entries = service.watch.as_ref().unwrap().previous.clone();
        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
            .unwrap();
        assert!(
            output.is_empty(),
            "a retry must not clear the client timeline"
        );
        assert_eq!(service.generation, generation);
        assert_eq!(service.watch.as_ref().unwrap().previous, entries);
        service.request_with_refresh(json!({"id":"h","method":"history","params":{"generation":generation,"before":"message:m7"}}).to_string().as_bytes(), &mut output, |_, _, _| Err("source_changing")).unwrap();
        let replies = frames(output);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["result"]["entries"].as_array().unwrap().len(), 7);
        assert_eq!(replies[0]["result"]["generation"], generation);
        cleanup(&service, root);
    }
    #[test]
    fn changing_new_identity_clears_previous_occupant_and_recovers_with_snapshot() {
        let (mut service, root, identity) = fixture();
        let generation = service.generation;
        *identity.lock().unwrap() = OTHER.to_owned();
        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["event"], "timeline.unavailable");
        assert_eq!(replies[0]["data"]["code"], "source_changing");
        assert!(service.watch.as_ref().unwrap().identity.is_none());
        assert!(service.watch.as_ref().unwrap().previous.is_empty());
        assert!(
            service
                .watch
                .as_ref()
                .unwrap()
                .follower
                .transcript
                .entries
                .is_empty()
        );
        let mut output = Vec::new();
        service.request(json!({"id":"h","method":"history","params":{"generation":generation,"before":"message:m7"}}).to_string().as_bytes(), &mut output).unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["event"], "timeline.snapshot");
        assert_eq!(replies[0]["data"]["identity"]["native_session_id"], OTHER);
        assert_eq!(replies[1]["error"]["code"], "stale_cursor");
        cleanup(&service, root);
    }
    #[test]
    fn counter_only_changes_emit_an_update() {
        let (mut service, root, _) = fixture();
        let generation = service.generation;
        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |follower, _, _| {
                follower.transcript.unknown_records += 1;
                Ok(true)
            })
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["event"], "timeline.updated");
        assert_eq!(replies[0]["data"]["entries"], json!([]));
        assert_eq!(replies[0]["data"]["unknown_records"], 1);
        assert_eq!(replies[0]["data"]["generation"], generation);
        cleanup(&service, root);
    }
    #[test]
    fn repeated_source_changing_signals_delay_while_keeping_history_and_recovers() {
        let (mut service, root, identity) = fixture();
        let generation = service.generation;
        let entries = service.watch.as_ref().unwrap().previous.clone();
        assert!(!entries.is_empty());

        for _ in 0..9 {
            let mut output = Vec::new();
            service
                .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
                .unwrap();
            assert!(output.is_empty());
            assert_eq!(service.generation, generation);
            assert_eq!(service.watch.as_ref().unwrap().previous, entries);
        }

        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["event"], "timeline.updated");
        assert_eq!(replies[0]["data"]["delayed"], true);
        assert_eq!(replies[0]["data"]["generation"], generation);
        assert_eq!(replies[0]["data"]["entries"], json!([]));
        assert_eq!(service.watch.as_ref().unwrap().previous, entries);

        let mut output = Vec::new();
        service
            .request_with_refresh(
                json!({"id":"h","method":"history","params":{"generation":generation,"before":"message:m7"}}).to_string().as_bytes(),
                &mut output,
                |_, _, _| Err("source_changing"),
            )
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["result"]["entries"].as_array().unwrap().len(), 7);
        assert_eq!(replies[0]["result"]["generation"], generation);

        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Ok(false))
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["event"], "timeline.updated");
        assert_eq!(replies[0]["data"]["delayed"], false);
        assert_eq!(service.generation, generation);

        *identity.lock().unwrap() = OTHER.to_owned();
        let mut output = Vec::new();
        service
            .poll_with_refresh(&mut output, |_, _, _| Err("source_changing"))
            .unwrap();
        let replies = frames(output);
        assert_eq!(replies[0]["event"], "timeline.unavailable");
        assert_eq!(replies[0]["data"]["code"], "source_changing");
        assert!(service.watch.as_ref().unwrap().identity.is_none());
        assert!(service.watch.as_ref().unwrap().previous.is_empty());

        cleanup(&service, root);
    }
}
