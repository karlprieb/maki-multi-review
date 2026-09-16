// Two test binaries include this module, so each compiles its own copy and sees
// the other's helpers as unused. The lint cannot apply here.
#![allow(dead_code)]

use std::borrow::Cow;
use std::path::Path;
use std::sync::{Arc, Mutex};

use maki_agent::tools::registry::{
    ExecFuture, HeaderFuture, HeaderResult, ParseError, Tool, ToolExecResult, ToolInvocation,
};
use maki_agent::tools::{DescriptionContext, ToolContext, ToolRegistry, ToolSource};
use maki_agent::{AgentMode, ToolOutput};
use maki_lua::{ModelRequest, PluginHost, PluginPermissions, SessionRequest, UiAction};
use serde_json::{Value, json};

pub const PLUGIN_NAME: &str = "maki-multi-review";
pub const TOOL: &str = "multi_review";

pub type Reply = Result<String, String>;

#[derive(Clone)]
struct StubState {
    replies: Arc<Mutex<Vec<Reply>>>,
    calls: Arc<Mutex<Vec<Value>>>,
    prompts: Arc<Mutex<Vec<String>>>,
    models: Arc<Mutex<Vec<Option<String>>>>,
}

impl StubState {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(replies)),
            calls: Arc::new(Mutex::new(Vec::new())),
            prompts: Arc::new(Mutex::new(Vec::new())),
            models: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn record(&self, input: &Value) -> Reply {
        let mut calls = self.calls.lock().unwrap();
        self.prompts
            .lock()
            .unwrap()
            .push(input["prompt"].as_str().unwrap_or_default().to_owned());
        self.models
            .lock()
            .unwrap()
            .push(input["model"].as_str().map(ToOwned::to_owned));
        calls.push(input.clone());
        let replies = self.replies.lock().unwrap();
        replies
            .get(calls.len() - 1)
            .or_else(|| replies.last())
            .cloned()
            .unwrap_or_else(|| Ok("default review".to_owned()))
    }
}

pub struct Harness {
    pub reg: Arc<ToolRegistry>,
    pub host: PluginHost,
    state: StubState,
    session_prompts: Arc<Mutex<Vec<String>>>,
    flashes: Arc<Mutex<Vec<String>>>,
    available_models: Arc<Mutex<Option<Vec<String>>>>,
    session_state: Arc<Mutex<Option<Value>>>,
    ui_drain_started: Arc<std::sync::atomic::AtomicBool>,
    permissions: Arc<maki_agent::permissions::PermissionManager>,
}

impl Harness {
    pub fn new_with_global_task(replies: Vec<Reply>, advertise_model: bool) -> Self {
        let state = StubState::new(replies);
        let reg = ToolRegistry::global_arc();
        let _ = reg.register(
            Arc::new(TaskStub {
                state: state.clone(),
                advertise_model,
            }),
            ToolSource::Lua {
                plugin: Arc::from("test-stub"),
            },
        );
        let host = PluginHost::new(Arc::clone(reg)).expect("host boots");
        host.load_package(
            PLUGIN_NAME,
            Path::new(env!("CARGO_MANIFEST_DIR")),
            PluginPermissions::trusted(),
            Default::default(),
        )
        .expect("package loads");
        Self::from_parts(Arc::clone(reg), host, state)
    }

    pub fn new(replies: Vec<Reply>) -> Self {
        let state = StubState::new(replies);
        let reg = Arc::new(ToolRegistry::new());
        reg.register(
            Arc::new(TaskStub {
                state: state.clone(),
                advertise_model: true,
            }),
            ToolSource::Lua {
                plugin: Arc::from("test-stub"),
            },
        )
        .expect("stub task registers");
        let host = PluginHost::new(Arc::clone(&reg)).expect("host boots");
        host.load_package(
            PLUGIN_NAME,
            Path::new(env!("CARGO_MANIFEST_DIR")),
            PluginPermissions::trusted(),
            Default::default(),
        )
        .expect("package loads");
        Self::from_parts(reg, host, state)
    }

    fn from_parts(reg: Arc<ToolRegistry>, host: PluginHost, state: StubState) -> Self {
        Self {
            reg,
            host,
            state,
            session_prompts: Arc::new(Mutex::new(Vec::new())),
            flashes: Arc::new(Mutex::new(Vec::new())),
            available_models: Arc::new(Mutex::new(None)),
            session_state: Arc::new(Mutex::new(None)),
            ui_drain_started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            permissions: Arc::new(maki_agent::permissions::PermissionManager::new(
                maki_config::PermissionsConfig {
                    default: maki_config::DefaultEffect::Allow,
                    rules: vec![],
                    ..Default::default()
                },
                std::path::PathBuf::from("/tmp"),
                maki_config::ProjectConfig::discover(std::path::Path::new("/tmp")),
                Arc::default(),
            )),
        }
    }

    pub fn set_reviewers(&self, entries: &str) {
        self.host
            .load_source(
                "reviewer_config",
                &format!(
                    "maki.api.set_slot(\"multireview.reviewers\", function() return {entries} end)"
                ),
            )
            .expect("reviewer layer loads");
    }

    pub fn answer_available_models(&self, models: Option<Vec<String>>) {
        *self.available_models.lock().unwrap() = models;
        self.start_ui_drain();
    }

    pub fn answer_session_thinking(&self, thinking: &str, options: Option<Vec<(&str, u32)>>) {
        let options = options.unwrap_or_else(|| {
            vec![
                ("off", 0),
                ("adaptive", 0),
                ("minimal", 3_276),
                ("low", 6_553),
                ("medium", 13_107),
                ("high", 19_660),
                ("xhigh", 26_214),
                ("max", 32_768),
            ]
        });
        let listed: Vec<Value> = options
            .iter()
            .map(|(name, tokens)| json!({ "name": name, "tokens": tokens }))
            .collect();
        *self.session_state.lock().unwrap() = Some(json!({
            "thinking": thinking,
            "thinking_options": listed,
            "supports_thinking": true,
            "fast": false,
        }));
        self.start_ui_drain();
    }

    fn start_ui_drain(&self) {
        use std::sync::atomic::Ordering;
        if self.ui_drain_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let rx = self.host.ui_action_rx();
        let prompts = Arc::clone(&self.session_prompts);
        let flashes = Arc::clone(&self.flashes);
        let models = Arc::clone(&self.available_models);
        let session = Arc::clone(&self.session_state);
        std::thread::spawn(move || {
            while let Ok(action) = rx.recv() {
                match action {
                    UiAction::Model { req, reply_tx } => {
                        let _ = reply_tx.send(match req {
                            ModelRequest::Available => match &*models.lock().unwrap() {
                                Some(list) => Ok(json!(list)),
                                None => Err("no UI".to_owned()),
                            },
                            ModelRequest::Get => match &*session.lock().unwrap() {
                                Some(state) => Ok(state.clone()),
                                None => Err("no UI".to_owned()),
                            },
                            _ => Err("unsupported request".to_owned()),
                        });
                    }
                    UiAction::Session {
                        req: SessionRequest::Prompt { text, .. },
                        reply_tx,
                    } => {
                        prompts.lock().unwrap().push(text);
                        let _ = reply_tx.send(Ok(json!(null)));
                    }
                    UiAction::Flash(msg) => flashes.lock().unwrap().push(msg),
                    _ => {}
                }
            }
        });
    }

    pub fn flashes(&self) -> Vec<String> {
        for _ in 0..50 {
            let seen = self.flashes.lock().unwrap().clone();
            if !seen.is_empty() {
                return seen;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        self.flashes.lock().unwrap().clone()
    }

    pub fn session_prompts(&self) -> Vec<String> {
        self.session_prompts.lock().unwrap().clone()
    }

    pub fn run_command(&self, plugin: &str, args: &str) {
        self.start_ui_drain();
        self.host.event_handle().run_command(
            Arc::from(plugin),
            Arc::from("/multi-review"),
            args.to_owned(),
            0,
        );
        for _ in 0..500 {
            if !self.session_prompts().is_empty() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("the command never sent a prompt");
    }

    pub fn command_names(&self) -> Vec<String> {
        self.host
            .command_reader()
            .load()
            .commands
            .iter()
            .map(|c| c.name.to_string())
            .collect()
    }

    pub fn task_prompts(&self) -> Vec<String> {
        self.state.prompts.lock().unwrap().clone()
    }

    pub fn model_args(&self) -> Vec<Option<String>> {
        self.state.models.lock().unwrap().clone()
    }

    pub fn task_input(&self, index: usize) -> Value {
        self.state.calls.lock().unwrap()[index].clone()
    }

    pub fn task_count(&self) -> usize {
        self.state.calls.lock().unwrap().len()
    }

    pub fn exec(&self, name: &str, input: Value) -> Result<String, String> {
        let entry = self
            .reg
            .get(name)
            .unwrap_or_else(|| panic!("tool {name} not registered"));
        let inv = entry.tool.parse(&input).expect("parse failed");
        let ctx = self.ctx();
        smol::block_on(async { inv.execute(&ctx).await })
            .output
            .map_or_else(Err, |out| match out {
                ToolOutput::Plain(s) | ToolOutput::Markdown(s) => Ok(s.text),
                other => panic!("unexpected output: {other:?}"),
            })
    }

    pub fn review(&self, context: &str) -> Result<String, String> {
        self.exec(TOOL, json!({ "context": context }))
    }

    fn ctx(&self) -> ToolContext {
        let (tx, _rx) = flume::unbounded();
        maki_agent::tools::interpreter_ctx(
            &AgentMode::Build,
            &maki_agent::EventSender::new(tx, 0),
            maki_agent::cancel::CancelToken::none(),
            Arc::clone(&self.permissions),
            maki_agent::tools::FileAccess::fresh(),
            None,
            Arc::clone(&self.reg),
        )
    }
}

struct TaskStub {
    state: StubState,
    advertise_model: bool,
}

impl Tool for TaskStub {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self, _ctx: &DescriptionContext) -> Cow<'_, str> {
        Cow::Borrowed("stub subagent")
    }

    fn schema(&self) -> Value {
        let mut properties = json!({
            "description": { "type": "string" },
            "prompt": { "type": "string" },
            "subagent_type": { "type": "string" },
        });
        if self.advertise_model {
            properties["model"] = json!({ "type": "string" });
        }
        json!({
            "type": "object",
            "properties": properties,
            "required": ["prompt"],
        })
    }

    fn parse(&self, input: &Value) -> Result<Box<dyn ToolInvocation>, ParseError> {
        Ok(Box::new(TaskInvocation {
            reply: self.state.record(input),
            header: input["description"].as_str().unwrap_or("task").to_owned(),
        }))
    }
}

struct TaskInvocation {
    reply: Reply,
    header: String,
}

impl ToolInvocation for TaskInvocation {
    fn start_header(&self) -> HeaderFuture {
        HeaderFuture::Ready(HeaderResult::plain(self.header.clone()))
    }

    fn execute(self: Box<Self>, _ctx: &ToolContext) -> ExecFuture<'_> {
        Box::pin(async move {
            ToolExecResult {
                output: self.reply.map(|text| ToolOutput::Plain(text.into())),
                annotation: None,
                written_path: None,
            }
        })
    }
}
