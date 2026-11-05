//! Per-run loopback transport for Claude's in-process Mod. The listener never
//! executes tools: the owning Agent loop drains calls and applies its policies.
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};

/// A native Claude subagent registered by the Mod's `agent.spawn` hook.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeAgent {
    pub agent_id: String,
    /// The `Agent` call that first started it. A resumed run keeps it: its
    /// calls belong to that card.
    pub parent_tool_use_id: String,
    /// The call that started this run: the `Agent` call itself, or the
    /// `SendMessage` that resumed it.
    pub invocation_tool_use_id: String,
    pub name: String,
    /// The `Agent` call's description (Claude repeats it for a resume).
    pub description: String,
    pub run_id: String,
    /// Runs without holding a Locus turn: an explicit background `Agent`, or
    /// any resumed run.
    pub background: bool,
}

/// An agent of this conversation started by an earlier Claude process. Its
/// native transcript survives, so `SendMessage` may resume it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownNativeAgent {
    pub agent_id: String,
    pub parent_tool_use_id: String,
    pub name: String,
}

struct AgentEntry {
    agent: NativeAgent,
    /// Its run ended (`turn.complete`); a resume starts a new run.
    finished: bool,
    /// Background runs registered and their task notifications seen. Each
    /// such run notifies once when it stops, in order.
    background_runs: u32,
    notices: u32,
}

/// Subagent callbacks never wait in the run queue: the session's agent pump
/// serves them whether or not a Locus run is active.
fn is_agent_callback(payload: &Value) -> bool {
    if !payload["agentId"].as_str().is_some_and(|id| !id.is_empty()) {
        return false;
    }
    match payload["type"].as_str() {
        Some(kind) => matches!(kind, "assistant" | "interrupted_assistant" | "tool_result" | "usage" | "agent_step"),
        None => payload["tool"].is_string() && payload["callId"].as_str().is_some_and(|id| !id.is_empty()),
    }
}

/// Host events about subagents that are not tied to one subagent's own
/// callbacks; the agent pump serves them too.
fn is_agent_event(payload: &Value) -> bool {
    matches!(payload["type"].as_str(), Some("agent_notification" | "control_done"))
}

/// Persisted next to the conversation's native transcript: which agent ids
/// belong to it (resumable with SendMessage) and which the user stopped.
#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct AgentRegistryFile {
    version: u32,
    #[serde(default)]
    agents: Vec<RegistryAgent>,
    #[serde(default)]
    stopped: Vec<String>,
}
#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct RegistryAgent {
    agent_id: String,
    parent_tool_use_id: String,
    name: String,
}

#[derive(Default)]
pub struct ControlSignals {
    events: std::sync::Mutex<std::collections::VecDeque<Value>>,
    changed: tokio::sync::Notify,
    interruptions: std::sync::atomic::AtomicU64,
    health: std::sync::Mutex<super::bridge_health::BridgeHealth>,
    /// Registered at ingress, never behind a pending tool receipt: a child's
    /// first call can arrive before its spawn hook returns.
    agents: std::sync::Mutex<std::collections::HashMap<String, AgentEntry>>,
    known_agents: std::sync::Mutex<std::collections::HashMap<String, KnownNativeAgent>>,
    /// Agents the user stopped. Never resumed automatically, also across
    /// processes (persisted in the registry file).
    stopped_agents: std::sync::Mutex<std::collections::HashSet<String>>,
    agent_queue: std::sync::Mutex<Option<mpsc::Sender<PendingCall>>>,
    /// Set once the session's agent pump consumes the subagent queue. Until
    /// then (native agents off) every callback stays on the run queue.
    agent_pump: std::sync::atomic::AtomicBool,
    registry_path: std::sync::Mutex<Option<std::path::PathBuf>>,
    agent_commands: std::sync::Mutex<Vec<Value>>,
}
impl ControlSignals {
    pub fn claim_agent_pump(&self) {
        self.agent_pump.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    fn agent_pump_claimed(&self) -> bool {
        self.agent_pump.load(std::sync::atomic::Ordering::SeqCst)
    }
    /// Loads and then maintains the conversation's agent registry.
    pub fn attach_registry(&self, path: std::path::PathBuf) {
        if let Some(file) = std::fs::read(&path).ok()
            .and_then(|bytes| serde_json::from_slice::<AgentRegistryFile>(&bytes).ok())
            .filter(|file| file.version == 1) {
            self.remember_native_agents(
                file.agents.into_iter().map(|agent| KnownNativeAgent {
                    agent_id: agent.agent_id, parent_tool_use_id: agent.parent_tool_use_id, name: agent.name,
                }),
                file.stopped,
            );
        }
        *self.registry_path.lock().unwrap_or_else(|e| e.into_inner()) = Some(path);
    }
    fn save_registry(&self) {
        let Some(path) = self.registry_path.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return; };
        let mut agents: std::collections::BTreeMap<String, RegistryAgent> = self.known_agents.lock().unwrap_or_else(|e| e.into_inner())
            .values().map(|agent| (agent.agent_id.clone(), RegistryAgent {
                agent_id: agent.agent_id.clone(), parent_tool_use_id: agent.parent_tool_use_id.clone(), name: agent.name.clone(),
            })).collect();
        for entry in self.agents.lock().unwrap_or_else(|e| e.into_inner()).values() {
            agents.insert(entry.agent.agent_id.clone(), RegistryAgent {
                agent_id: entry.agent.agent_id.clone(), parent_tool_use_id: entry.agent.parent_tool_use_id.clone(), name: entry.agent.name.clone(),
            });
        }
        let file = AgentRegistryFile { version: 1, agents: agents.into_values().collect(), stopped: self.stopped_native_agents() };
        let Ok(bytes) = serde_json::to_vec(&file) else { return; };
        let temporary = path.with_extension("json.tmp");
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(&temporary, bytes))
            .and_then(|_| std::fs::rename(&temporary, &path));
        if let Err(error) = written {
            eprintln!("[Claude] failed to save the native agent registry: {error}");
        }
    }
    /// Answered at ingress: SendMessage/TaskStop routing must not wait for a
    /// run or another agent's call.
    fn agent_route(&self, payload: &Value) -> Value {
        let id = payload["id"].as_str().unwrap_or_default();
        json!({"ok":true,"native":!id.is_empty() && self.is_native_agent(id),
            "running":self.native_agent_running(id),"stopped":self.native_agent_stopped(id)})
    }
    pub fn native_agent(&self, id: &str) -> Option<NativeAgent> {
        self.agents.lock().unwrap_or_else(|e| e.into_inner()).get(id).map(|entry| entry.agent.clone())
    }
    pub async fn wait_native_agent(&self, id: &str) -> NativeAgent {
        loop {
            let changed = self.changed.notified();
            if let Some(agent) = self.native_agent(id) { return agent; }
            changed.await;
        }
    }
    /// A started run that has not finished.
    pub fn native_agent_running(&self, id: &str) -> bool {
        self.agents.lock().unwrap_or_else(|e| e.into_inner()).get(id).is_some_and(|entry| !entry.finished)
    }
    /// A resumed run's first request can precede its registration.
    pub async fn wait_agent_running(&self, id: &str) {
        loop {
            let changed = self.changed.notified();
            if self.native_agent_running(id) { return; }
            changed.await;
        }
    }
    /// Consumes one task notification of the agent: whether it belongs to an
    /// earlier run, Claude having already continued the agent (a message
    /// queued for it, or SendMessage) before Locus saw it.
    pub fn take_agent_notice(&self, id: &str) -> bool {
        let mut agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        let Some(entry) = agents.get_mut(id) else { return false; };
        entry.notices += 1;
        entry.notices < entry.background_runs
    }
    /// Background work keeps the Claude process alive between Locus runs.
    pub fn has_running_background_agents(&self) -> bool {
        self.agents.lock().unwrap_or_else(|e| e.into_inner()).values()
            .any(|entry| entry.agent.background && !entry.finished)
    }
    pub fn running_background_agents(&self) -> Vec<NativeAgent> {
        self.agents.lock().unwrap_or_else(|e| e.into_inner()).values()
            .filter(|entry| entry.agent.background && !entry.finished)
            .map(|entry| entry.agent.clone()).collect()
    }
    /// Whether `SendMessage`/`TaskStop`/`TaskOutput` may address it natively.
    pub fn is_native_agent(&self, id: &str) -> bool {
        self.native_agent(id).is_some() || self.known_agents.lock().unwrap_or_else(|e| e.into_inner()).contains_key(id)
    }
    pub fn remember_native_agents(&self, agents: impl IntoIterator<Item = KnownNativeAgent>, stopped: impl IntoIterator<Item = String>) {
        let mut known = self.known_agents.lock().unwrap_or_else(|e| e.into_inner());
        for agent in agents { known.insert(agent.agent_id.clone(), agent); }
        drop(known);
        self.stopped_agents.lock().unwrap_or_else(|e| e.into_inner()).extend(stopped);
    }
    pub fn native_agent_stopped(&self, id: &str) -> bool {
        self.stopped_agents.lock().unwrap_or_else(|e| e.into_inner()).contains(id)
    }
    pub fn stop_native_agent(&self, id: &str) {
        self.stopped_agents.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string());
        self.save_registry();
        self.changed.notify_waiters();
    }
    pub async fn wait_agent_stopped(&self, id: &str) {
        loop {
            let changed = self.changed.notified();
            if self.native_agent_stopped(id) { return; }
            changed.await;
        }
    }
    /// A user stop: the persisted flag, and Claude's TaskStop for a running
    /// agent, which the Mod runs from its input poll (`agentControl`).
    pub fn request_agent_stop(&self, id: &str) {
        self.stop_native_agent(id);
        if !self.native_agent_running(id) { return; }
        let mut commands = self.agent_commands.lock().unwrap_or_else(|e| e.into_inner());
        if !commands.iter().any(|command| command["agentId"] == id) {
            commands.push(json!({"id":format!("stop:{id}:{}", uuid::Uuid::new_v4().simple()),"type":"stop","agentId":id}));
        }
    }
    pub fn control_done(&self, id: &str) {
        self.agent_commands.lock().unwrap_or_else(|e| e.into_inner()).retain(|command| command["id"] != id);
    }
    /// `/options` "agentControl": pending commands and the stopped agents.
    pub fn agent_control(&self) -> Value {
        json!({"commands":self.agent_commands.lock().unwrap_or_else(|e| e.into_inner()).clone(),
            "stopped":self.stopped_native_agents()})
    }
    pub fn stopped_native_agents(&self) -> Vec<String> {
        let mut stopped: Vec<_> = self.stopped_agents.lock().unwrap_or_else(|e| e.into_inner()).iter().cloned().collect();
        stopped.sort();
        stopped
    }
    fn set_agent_queue(&self, queue: mpsc::Sender<PendingCall>) {
        *self.agent_queue.lock().unwrap_or_else(|e| e.into_inner()) = Some(queue);
    }
    /// Internal events (not Mod callbacks) the agent pump must observe.
    fn push_agent_event(&self, payload: Value) {
        if !self.agent_pump_claimed() { return; }
        let Some(queue) = self.agent_queue.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return; };
        let (reply, _) = oneshot::channel();
        forward(queue, PendingCall { payload, reply, received_at: tokio::time::Instant::now() });
    }
    fn register_native_agent(&self, payload: &Value, options: &Arc<std::sync::RwLock<Value>>) -> Value {
        let resume = payload["resume"] == true;
        // A new agent starts only inside a Locus turn (its Agent call). Claude
        // continues an agent of this conversation with SendMessage, or later
        // with a message queued for it, as a background run needing no turn.
        let active = {
            let options = options.read().unwrap_or_else(|e| e.into_inner());
            options["active"] == true && payload["runId"] == options["inputId"]
        };
        if !resume && !active {
            return json!({"deny":"Claude subagents can start only during an active Locus turn."});
        }
        let text = |key: &str| payload[key].as_str().filter(|value| !value.is_empty()).map(str::to_owned);
        let (Some(agent_id), Some(invocation)) = (text("agentId"), text("parentToolUseId")) else {
            return json!({"deny":"Invalid Claude subagent registration"});
        };
        if self.native_agent_stopped(&agent_id) {
            return json!({"deny":"The user stopped this agent; it stays stopped in this conversation."});
        }
        let mut agents = self.agents.lock().unwrap_or_else(|e| e.into_inner());
        let previous = agents.get(&agent_id).map(|entry| (entry.agent.clone(), entry.finished, entry.background_runs, entry.notices));
        let known = self.known_agents.lock().unwrap_or_else(|e| e.into_inner()).get(&agent_id).cloned();
        let (parent_tool_use_id, name) = if resume {
            match (&previous, known) {
                (Some((agent, ..)), _) => (agent.parent_tool_use_id.clone(), agent.name.clone()),
                (None, Some(known)) => (known.parent_tool_use_id, known.name),
                (None, None) => return json!({"deny":"Claude resumed an agent this conversation never started"}),
            }
        } else {
            if previous.as_ref().is_some_and(|(agent, ..)| agent.parent_tool_use_id != invocation) {
                return json!({"deny":"Claude reused a subagent identity for another Agent call"});
            }
            (invocation.clone(), text("name").unwrap_or_default())
        };
        let Some(run_id) = text("runId").or_else(|| previous.as_ref().map(|(agent, ..)| agent.run_id.clone())) else {
            return json!({"deny":"Invalid Claude subagent registration"});
        };
        let description = text("description")
            .or_else(|| previous.as_ref().map(|(agent, ..)| agent.description.clone()))
            .unwrap_or_default();
        let agent = NativeAgent {
            agent_id: agent_id.clone(), parent_tool_use_id, invocation_tool_use_id: invocation,
            name: text("name").unwrap_or(name), description, run_id,
            background: resume || payload["background"] == true,
        };
        // A lost receipt repeats a registration; it starts nothing new.
        let repeated = previous.as_ref().is_some_and(|(known, finished, ..)| known == &agent && !finished);
        let first = previous.is_none();
        let (runs, notices) = previous.as_ref().map_or((0, 0), |(_, _, runs, notices)| (*runs, *notices));
        let background_runs = runs + u32::from(agent.background && !repeated);
        agents.insert(agent_id.clone(), AgentEntry { agent, finished: false, background_runs, notices });
        drop(agents);
        if first { self.save_registry(); }
        self.changed.notify_waiters();
        if !repeated { self.push_agent_event(json!({"type":"agent_registered","agentId":agent_id})); }
        json!({"ok":true})
    }
    fn finish_native_agent(&self, payload: &Value) -> Value {
        let Some(id) = payload["agentId"].as_str() else { return json!({"deny":"Missing Claude subagent identity"}); };
        if let Some(entry) = self.agents.lock().unwrap_or_else(|e| e.into_inner()).get_mut(id) {
            entry.finished = true;
        }
        self.changed.notify_waiters();
        json!({"ok":true})
    }
    pub fn begin_turn(&self, id: &str) {
        self.health.lock().unwrap_or_else(|e| e.into_inner()).begin_turn(id, std::time::Instant::now());
    }
    #[cfg(test)]
    pub fn health_error(&self, id: &str) -> Option<String> {
        self.health_error_with_transport(id, None)
    }
    pub fn health_error_with_transport(&self, id: &str, transport_failure: Option<&str>) -> Option<String> {
        self.health.lock().unwrap_or_else(|e| e.into_inner())
            .error_with_transport(id, std::time::Instant::now(), transport_failure)
    }
    pub fn route(&self, payload:&Value, options:&Arc<std::sync::RwLock<Value>>) -> Option<Value> {
        if payload["ready"] == true {
            let id = payload["sessionId"].as_str()?;
            let active = options.read().unwrap_or_else(|e|e.into_inner())["active"] == true;
            return match self.health.lock().unwrap_or_else(|e|e.into_inner()).worker_ready(id, active) {
                Ok(true) => None,
                Ok(false) => Some(json!({"ok":true})),
                Err(error) => Some(json!({"deny":error})),
            };
        }
        // A live, scoped hook callback is also proof of Mod liveness. Do this
        // at ingress, not in the consumer, which can be awaiting a long tool.
        // Configuration/poll requests and stale/unknown events are not proof.
        if is_turn_activity(payload) {
            let options = options.read().unwrap_or_else(|e| e.into_inner());
            if options["active"] == true && payload["runId"] == options["inputId"] {
                if let Some(id) = payload["runId"].as_str() {
                    self.health.lock().unwrap_or_else(|e| e.into_inner()).activity(id, std::time::Instant::now());
                }
            }
        }
        if payload["type"] == "agent_spawn" {
            return Some(self.register_native_agent(payload, options));
        }
        if payload["type"] == "agent_finished" {
            return Some(self.finish_native_agent(payload));
        }
        if payload["type"] == "agent_route" {
            return Some(self.agent_route(payload));
        }
        // A subagent's interrupted call is scoped to that agent, not to the
        // Locus turn: a background agent's calls outlive it. It never
        // interrupts the main agent's calls.
        if payload["type"] == "turn_interrupted" && payload["agentId"].as_str().is_some_and(|id| !id.is_empty()) {
            if !payload["agentId"].as_str().is_some_and(|id| self.native_agent(id).is_some()) {
                return Some(json!({"deny":"Unknown Claude subagent"}));
            }
            if !payload["id"].as_str().is_some_and(|id| id.starts_with("agent:")) {
                return Some(json!({"deny":"Invalid Claude subagent interruption scope"}));
            }
            let mut events = self.events.lock().unwrap_or_else(|e| e.into_inner());
            if events.len() >= 64 { events.pop_front(); }
            events.push_back(payload.clone());
            drop(events);
            self.changed.notify_waiters();
            return Some(json!({"ok":true}));
        }
        if matches!(payload["type"].as_str(), Some("bridge_health" | "bridge_fault")) {
            let options = options.read().unwrap_or_else(|e|e.into_inner());
            if options["active"] != true || payload["runId"] != options["inputId"] {
                return Some(json!({"deny":"Stale Claude bridge health"}));
            }
            let id = payload["runId"].as_str().unwrap_or_default();
            let mut health = self.health.lock().unwrap_or_else(|e|e.into_inner());
            if payload["type"] == "bridge_fault" {
                health.fault(id, payload["error"].as_str().unwrap_or("Native hook failed"));
            } else {
                health.observe(id, payload["state"].as_str().unwrap_or_default(), std::time::Instant::now());
            }
            return Some(json!({"ok":true}));
        }
        if !matches!(payload["type"].as_str(),Some("input_ready"|"input_images"|"input_staging_error"|"input_queued"|"input_consumed"|"turn_interrupted")) {return None;}
        let options=options.read().unwrap_or_else(|e|e.into_inner());
        if options["active"]!=true || payload["runId"]!=options["inputId"] {return Some(json!({"deny":"Stale Claude input control"}));}
        if matches!(payload["type"].as_str(),Some("input_ready"|"input_images"|"input_staging_error"))
            && (payload["id"].as_str().is_none() || (payload["id"]!=options["inputId"] && payload["id"]!=options["insertion"]["inputId"])) {
            return Some(json!({"deny":"Stale Claude input preparation"}));
        }
        let mut events=self.events.lock().unwrap_or_else(|e|e.into_inner());
        if events.len()>=64 {events.pop_front();}
        events.push_back(payload.clone());
        if payload["type"]=="turn_interrupted" {self.interruptions.fetch_add(1,std::sync::atomic::Ordering::SeqCst);}
        self.changed.notify_waiters();
        Some(json!({"ok":true}))
    }
    pub fn contains(&self,kind:&str,id:&str,count:Option<u64>)->bool {
        self.events.lock().unwrap_or_else(|e|e.into_inner()).iter().any(|event|event["type"]==kind && event["id"]==id && count.is_none_or(|count|event["count"].as_u64()==Some(count)))
    }
    pub async fn wait(&self,kind:&str,id:&str,count:Option<u64>){
        loop{let changed=self.changed.notified();if self.contains(kind,id,count){return;}changed.await;}
    }
    pub async fn input_receipt(&self, id: &str) -> Result<Value, String> {
        loop {
            let changed = self.changed.notified();
            if let Some(receipt) = self.events.lock().unwrap_or_else(|e|e.into_inner()).iter()
                .find(|event|event["type"]=="input_consumed" && event["id"]==id).cloned() {
                super::recovery::validate_input_checkpoint(&receipt["checkpoint"])?;
                return Ok(receipt["checkpoint"].clone());
            }
            changed.await;
        }
    }
    pub async fn wait_input(&self,kind:&str,id:&str,count:Option<u64>)->Result<(),String> {
        loop {
            let changed=self.changed.notified();
            {
                let events=self.events.lock().unwrap_or_else(|e|e.into_inner());
                if let Some(error)=events.iter().find(|event|event["type"]=="input_staging_error" && event["id"]==id) {
                    return Err(error["error"].as_str().unwrap_or("Claude could not prepare image attachments").to_string());
                }
                if events.iter().any(|event|event["type"]==kind && event["id"]==id && count.is_none_or(|count|event["count"].as_u64()==Some(count))) {
                    return Ok(());
                }
            }
            changed.await;
        }
    }
    pub fn interruption_epoch(&self)->u64 {self.interruptions.load(std::sync::atomic::Ordering::SeqCst)}
    pub async fn interrupted_after(&self,epoch:u64){
        loop{let changed=self.changed.notified();if self.interruption_epoch()!=epoch{return;}changed.await;}
    }
}
fn is_turn_activity(payload: &Value) -> bool {
    matches!(payload["type"].as_str(), Some(
        "input_ready" | "input_images" | "input_staging_error" | "input_queued" | "input_consumed" |
        "turn_interrupted" | "input_error" | "turn_start" | "step_start" | "phase" | "text" |
        "thinking" | "assistant" | "interrupted_assistant" | "tool_start" | "tool_result" |
        "checkpoint" | "usage" | "diagnostic" | "complete" | "compact_start" | "compact_prepare" | "compact_done" |
        "command_start" | "command"
    )) || (payload["type"].is_null() && payload["tool"].is_string()
        && payload["callId"].as_str().is_some_and(|id| !id.is_empty()) && payload["input"].is_object())
}

type Receipts =
    Arc<tokio::sync::Mutex<std::collections::HashMap<String, oneshot::Receiver<Value>>>>;

fn options_view(options: &Value, view: Option<&str>) -> Value {
    match view {
        Some("input") => {
            let mut value = json!({"active":options["active"],"inputId":options["inputId"],"terminalText":options["terminalText"],"imageCount":options["imageCount"]});
            if options["insertion"].is_object() {
                value["insertion"] = json!({"inputId":options["insertion"]["inputId"],"terminalText":options["insertion"]["terminalText"],"imageCount":options["insertion"]["imageCount"]});
            }
            // Subagent stops travel with the existing input poll, so SSH
            // hosts answer them from the configured options as well.
            if options["agentControl"].is_object() {
                value["agentControl"] = options["agentControl"].clone();
            }
            value
        },
        Some("prompt") => json!({"systemPrompt":options["systemPrompt"]}),
        Some("session") => json!({"active":options["active"],"attachmentMappings":options["attachmentMappings"]}),
        _ => options.clone(),
    }
}

pub use locus_remote_core::claude_ssh::PendingCall;

pub struct ModBridge {
    pub url: String,
    pub token: String,
    pub calls: mpsc::Receiver<PendingCall>,
    /// Subagent callbacks, taken by the session's agent pump.
    pub agent_calls: Option<mpsc::Receiver<PendingCall>>,
    pub options: Arc<std::sync::RwLock<Value>>,
    pub controls: Arc<ControlSignals>,
    task: Option<JoinHandle<()>>,
}

/// A full queue holds back only this call; the other queue keeps flowing.
/// A Mod hook sends its next callback only after this one's receipt, so a
/// waiting call is never overtaken by its own successor.
fn forward(queue: mpsc::Sender<PendingCall>, call: PendingCall) {
    let refuse = |call: PendingCall| {
        let _ = call.reply.send(json!({"callId":call.payload["callId"],
            "deny":"Locus run is closed or busy; do not automatically retry"}));
    };
    match queue.try_send(call) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(call)) => {
            tokio::spawn(async move {
                if let Err(error) = queue.send(call).await { refuse(error.0); }
            });
        }
        Err(mpsc::error::TrySendError::Closed(call)) => refuse(call),
    }
}

/// Every transport submits into one ingress queue. Once the session's agent
/// pump claims them, subagent callbacks move to their own queue: they must
/// not wait behind, or for, a Locus run.
fn split_calls(controls: &Arc<ControlSignals>) -> (mpsc::Sender<PendingCall>, mpsc::Receiver<PendingCall>, mpsc::Receiver<PendingCall>) {
    let (ingress, mut incoming) = mpsc::channel::<PendingCall>(16);
    let (run_send, run_calls) = mpsc::channel(16);
    let (agent_send, agent_calls) = mpsc::channel(64);
    controls.set_agent_queue(agent_send.clone());
    let controls = controls.clone();
    tokio::spawn(async move {
        while let Some(call) = incoming.recv().await {
            let agent = controls.agent_pump_claimed()
                && (is_agent_callback(&call.payload) || is_agent_event(&call.payload));
            forward(if agent { agent_send.clone() } else { run_send.clone() }, call);
        }
    });
    (ingress, run_calls, agent_calls)
}

impl ModBridge {
    /// SSH supplies the same typed calls without opening a local network port.
    pub fn channel() -> (Self, mpsc::Sender<PendingCall>) {
        let controls = Arc::new(ControlSignals::default());
        let (send, calls, agent_calls) = split_calls(&controls);
        (Self {
            url: String::new(), token: uuid::Uuid::new_v4().to_string(), calls, agent_calls: Some(agent_calls),
            options: Arc::new(std::sync::RwLock::new(json!({"transport":"tui","active":false}))),
            controls,
            task: None,
        }, send)
    }

    pub async fn start() -> Result<Self, String> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let token = uuid::Uuid::new_v4().to_string();
        let auth = Arc::new(format!("Bearer {token}"));
        let expected_host = Arc::new(address.to_string());
        let options = Arc::new(std::sync::RwLock::new(
            json!({"transport":"stream","active":true}),
        ));
        let shared_options = options.clone();
        let controls=Arc::new(ControlSignals::default());
        let (send, calls, agent_calls) = split_calls(&controls);
        let shared_controls=controls.clone();
        let receipts: Receipts =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let task = tokio::spawn(async move {
            // JoinSet aborts all accepted connections when the run drops.
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{
                        let Ok((stream,_))=accepted else { break; };
                        // Stream callbacks exchange tiny headers/body frames.
                        // Avoid Nagle/delayed-ACK stalls on the loopback bridge.
                        let _ = stream.set_nodelay(true);
                        if connections.len()>=32 { drop(stream); continue; }
                        let auth=auth.clone();
                        let expected_host=expected_host.clone();
                        let send=send.clone();
                        let options=shared_options.clone();
                        let receipts=receipts.clone();
                        let controls=shared_controls.clone();
                        connections.spawn(async move {
                            let service=hyper::service::service_fn(move |req| {
                                handle(req,auth.clone(),expected_host.clone(),send.clone(),options.clone(),receipts.clone(),controls.clone())
                            });
                            let _=hyper::server::conn::http1::Builder::new()
                                .serve_connection(TokioIo::new(stream),service).await;
                        });
                    }
                    _=connections.join_next(),if !connections.is_empty()=>{}
                }
            }
        });
        Ok(Self {
            url: format!("http://{address}"),
            token,
            calls,
            agent_calls: Some(agent_calls),
            options,
            controls,
            task: Some(task),
        })
    }
}

impl Drop for ModBridge {
    fn drop(&mut self) {
        if let Some(task) = &self.task { task.abort(); }
    }
}

fn response(status: StatusCode, value: Value) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Full::new(Bytes::from(value.to_string())))
        .expect("static HTTP response")
}

async fn handle(
    req: Request<Incoming>,
    auth: Arc<String>,
    host: Arc<String>,
    send: mpsc::Sender<PendingCall>,
    options: Arc<std::sync::RwLock<Value>>,
    receipts: Receipts,
    controls: Arc<ControlSignals>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let fail = |status, message| Ok(response(status, json!({"error":message})));
    if req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        != Some(auth.as_str())
    {
        return fail(StatusCode::UNAUTHORIZED, "Unauthorized");
    }
    if req.headers().contains_key("origin")
        || req.headers().get("host").and_then(|h| h.to_str().ok()) != Some(host.as_str())
    {
        return fail(StatusCode::FORBIDDEN, "Invalid origin or host");
    }
    let ready = req.uri().path() == "/ready";
    let event = req.uri().path() == "/event";
    let configuration = req.uri().path() == "/options";
    let polling = req.uri().path() == "/poll";
    if req.method() != Method::POST
        || (!ready && !event && !configuration && !polling && req.uri().path() != "/call")
        || req.uri().query().is_some()
    {
        return fail(StatusCode::NOT_FOUND, "Unknown bridge endpoint");
    }
    let body = match tokio::time::timeout(
        std::time::Duration::from_secs(10),
        Limited::new(req.into_body(), 1024 * 1024).collect(),
    )
    .await
    {
        Ok(Ok(body)) => body.to_bytes(),
        _ => {
            return fail(
                StatusCode::PAYLOAD_TOO_LARGE,
                "Invalid or oversized request body",
            )
        }
    };
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => return fail(StatusCode::BAD_REQUEST, "Invalid JSON"),
    };
    if payload["protocol"] != 1 {
        return fail(StatusCode::BAD_REQUEST, "Invalid bridge protocol");
    }
    let payload = if ready {
        json!({"ready":true,"sessionId":payload["sessionId"],"model":payload["model"]})
    } else { payload };
    if let Some(reply)=controls.route(&payload,&options) {return Ok(response(StatusCode::OK,reply));}
    if polling {
        let Some(ticket) = payload["ticket"].as_str() else {
            return fail(StatusCode::BAD_REQUEST, "Missing receipt");
        };
        let Some(receiver) = receipts.lock().await.remove(ticket) else {
            return fail(
                StatusCode::GONE,
                "Receipt expired or already consumed; do not resubmit the operation",
            );
        };
        return Ok(receipt_response(
            receipts,
            ticket.to_string(),
            receiver,
            std::time::Duration::from_secs(10),
        )
        .await);
    }
    if configuration {
        return Ok(response(
            StatusCode::OK,
            options_view(&options.read().unwrap_or_else(|e| e.into_inner()), payload["view"].as_str()),
        ));
    }
    if (!ready && payload["protocol"] != 1)
        || (!ready
            && !event
            && (!payload["tool"].is_string()
                || !payload["callId"].as_str().is_some_and(|id| !id.is_empty())
                || !payload["input"].is_object()))
    {
        return fail(StatusCode::BAD_REQUEST, "Invalid tool call envelope");
    }
    let (reply, receive) = oneshot::channel();
    // Parallel subagent calls each hold a receipt until the Mod polls it.
    if receipts.lock().await.len() >= 256 {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "Too many unconsumed local results",
        );
    }
    if send
        .try_send(PendingCall {
            payload,
            reply,
            received_at: tokio::time::Instant::now(),
        })
        .is_err()
    {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "Locus run is closed or busy; do not automatically retry",
        );
    }
    Ok(receipt_response(
        receipts,
        uuid::Uuid::new_v4().to_string(),
        receive,
        std::time::Duration::from_millis(100),
    )
    .await)
}

async fn receipt_response(
    receipts: Receipts,
    ticket: String,
    mut receiver: oneshot::Receiver<Value>,
    wait: std::time::Duration,
) -> Response<Full<Bytes>> {
    match tokio::time::timeout(wait, &mut receiver).await {
        Ok(Ok(value)) => response(StatusCode::OK, value),
        Ok(Err(_)) => response(
            StatusCode::GONE,
            json!({"error":"Locus run ended; execution outcome may be unknown"}),
        ),
        Err(_) => {
            receipts.lock().await.insert(ticket.clone(), receiver);
            response(StatusCode::ACCEPTED, json!({"pending":ticket}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_agents_register_only_in_the_active_turn_and_keep_their_parent_call() {
        let controls = ControlSignals::default();
        let options = Arc::new(std::sync::RwLock::new(json!({"active":true,"inputId":"run"})));
        let spawn = |agent: &str, parent: &str, run: &str| json!({"type":"agent_spawn","runId":run,"agentId":agent,"parentToolUseId":parent,"name":"Explore"});
        let waiting = controls.wait_native_agent("a1");
        let register = async { controls.route(&spawn("a1", "toolu_1", "run"), &options) };
        let (agent, reply) = tokio::join!(waiting, register);
        assert_eq!(reply, Some(json!({"ok":true})));
        assert_eq!(agent, NativeAgent { agent_id: "a1".into(), parent_tool_use_id: "toolu_1".into(),
            invocation_tool_use_id: "toolu_1".into(), name: "Explore".into(), description: String::new(), run_id: "run".into(), background: false });
        // The same registration is idempotent; another parent call is not.
        assert_eq!(controls.route(&spawn("a1", "toolu_1", "run"), &options), Some(json!({"ok":true})));
        assert!(controls.route(&spawn("a1", "toolu_2", "run"), &options).unwrap()["deny"].is_string());
        assert_eq!(controls.native_agent("a1").unwrap().parent_tool_use_id, "toolu_1");
        // Stale turns and idle sessions cannot start subagents.
        assert!(controls.route(&spawn("a2", "toolu_3", "old"), &options).unwrap()["deny"].is_string());
        *options.write().unwrap() = json!({"active":false,"inputId":"run"});
        assert!(controls.route(&spawn("a3", "toolu_4", "run"), &options).unwrap()["deny"].is_string());
        assert!(controls.route(&json!({"type":"agent_spawn","runId":"run","agentId":"","parentToolUseId":"toolu_5"}), &options).unwrap()["deny"].is_string());
        assert!(controls.native_agent("a2").is_none() && controls.native_agent("a3").is_none());
    }

    #[test]
    fn background_and_resumed_runs_keep_their_origin_and_respect_user_stops() {
        let root = tempfile::tempdir().unwrap();
        let registry = root.path().join("native_agents.json");
        let controls = ControlSignals::default();
        controls.attach_registry(registry.clone());
        let options = Arc::new(std::sync::RwLock::new(json!({"active":true,"inputId":"run"})));
        let spawn = json!({"type":"agent_spawn","runId":"run","agentId":"bg","parentToolUseId":"toolu_agent","name":"Explore","background":true});
        assert_eq!(controls.route(&spawn, &options), Some(json!({"ok":true})));
        assert!(controls.has_running_background_agents());
        assert_eq!(controls.route(&json!({"type":"agent_route","id":"bg"}), &options).unwrap(),
            json!({"ok":true,"native":true,"running":true,"stopped":false}));
        assert_eq!(controls.route(&json!({"type":"agent_finished","agentId":"bg"}), &options), Some(json!({"ok":true})));
        assert!(!controls.has_running_background_agents());
        // SendMessage resumes it: a new background run of the same card.
        let resume = json!({"type":"agent_spawn","runId":"run","agentId":"bg","parentToolUseId":"toolu_send","name":"Explore","resume":true});
        assert_eq!(controls.route(&resume, &options), Some(json!({"ok":true})));
        let agent = controls.native_agent("bg").unwrap();
        assert_eq!((agent.parent_tool_use_id.as_str(), agent.invocation_tool_use_id.as_str(), agent.background), ("toolu_agent", "toolu_send", true));
        // A resume of an agent this conversation never started is refused.
        let unknown = json!({"type":"agent_spawn","runId":"run","agentId":"other","parentToolUseId":"toolu_x","resume":true});
        assert!(controls.route(&unknown, &options).unwrap()["deny"].is_string());
        // A user stop survives a new process and blocks automatic resumes.
        controls.stop_native_agent("bg");
        let restarted = ControlSignals::default();
        restarted.attach_registry(registry);
        assert!(restarted.is_native_agent("bg") && restarted.native_agent_stopped("bg"));
        assert_eq!(restarted.route(&json!({"type":"agent_route","id":"bg"}), &options).unwrap(),
            json!({"ok":true,"native":true,"running":false,"stopped":true}));
        assert!(restarted.route(&resume, &options).unwrap()["deny"].as_str().unwrap().contains("stopped"));
        assert_eq!(restarted.route(&json!({"type":"agent_route","id":"nobody"}), &options).unwrap()["native"], false);
    }

    #[tokio::test]
    async fn claude_continues_agents_while_idle_and_an_earlier_run_never_closes_the_newer_one() {
        let controls = ControlSignals::default();
        let options = Arc::new(std::sync::RwLock::new(json!({"active":true,"inputId":"run"})));
        let spawn = json!({"type":"agent_spawn","runId":"run","agentId":"bg","parentToolUseId":"toolu_agent","name":"Explore","background":true});
        assert_eq!(controls.route(&spawn, &options), Some(json!({"ok":true})));
        assert_eq!(controls.route(&json!({"type":"agent_finished","agentId":"bg"}), &options), Some(json!({"ok":true})));
        // A message queued for the agent continues it after the turn ended;
        // its first request waits for that registration.
        *options.write().unwrap() = json!({"active":false,"inputId":null});
        let resume = json!({"type":"agent_spawn","runId":null,"agentId":"bg","parentToolUseId":"toolu_send","name":"Explore","resume":true});
        let (_, reply) = tokio::join!(controls.wait_agent_running("bg"), async { controls.route(&resume, &options) });
        assert_eq!(reply, Some(json!({"ok":true})));
        assert_eq!(controls.native_agent("bg").unwrap().run_id, "run");
        // A new agent still needs an active turn.
        let fresh = json!({"type":"agent_spawn","runId":"run","agentId":"new","parentToolUseId":"toolu_new","name":"Explore"});
        assert!(controls.route(&fresh, &options).unwrap()["deny"].is_string());
        // The first run's notification, seen after the resume, is superseded.
        assert!(controls.take_agent_notice("bg"));
        assert!(!controls.take_agent_notice("bg"));
        assert!(!controls.take_agent_notice("nobody"));
        // A subagent's interruption ends only its own scope.
        let interrupt = |agent: &str| json!({"type":"turn_interrupted","runId":"run","agentId":agent,"id":format!("agent:{agent}:1")});
        assert!(controls.route(&interrupt("nobody"), &options).unwrap()["deny"].is_string());
        assert_eq!(controls.route(&interrupt("bg"), &options), Some(json!({"ok":true})));
        assert!(controls.contains("turn_interrupted", "agent:bg:1", None));
        assert_eq!(controls.interruption_epoch(), 0);
    }

    #[tokio::test]
    async fn subagent_callbacks_use_their_own_queue_only_after_the_pump_claims_it() {
        let (mut bridge, send) = ModBridge::channel();
        let call = |payload: Value| {
            let (reply, receive) = oneshot::channel();
            (PendingCall { payload, reply, received_at: tokio::time::Instant::now() }, receive)
        };
        let subagent = json!({"tool":"Read","callId":"c1","agentId":"a1","input":{}});
        // Native agents off: everything stays on the run queue.
        send.send(call(subagent.clone()).0).await.unwrap();
        assert_eq!(bridge.calls.recv().await.unwrap().payload, subagent);
        bridge.controls.claim_agent_pump();
        let mut agent_calls = bridge.agent_calls.take().unwrap();
        let main = json!({"tool":"Read","callId":"c2","agentId":null,"input":{}});
        send.send(call(subagent.clone()).0).await.unwrap();
        send.send(call(main.clone()).0).await.unwrap();
        send.send(call(json!({"type":"agent_notification","agentId":"a1","status":"completed"})).0).await.unwrap();
        assert_eq!(agent_calls.recv().await.unwrap().payload, subagent);
        assert_eq!(bridge.calls.recv().await.unwrap().payload, main);
        assert_eq!(agent_calls.recv().await.unwrap().payload["type"], "agent_notification");
        // A busy run queue never holds back a subagent.
        let mut waiting = Vec::new();
        for index in 0..20 {
            let (pending, receive) = call(json!({"type":"text","runId":"run","text":index.to_string()}));
            send.send(pending).await.unwrap();
            waiting.push(receive);
        }
        send.send(call(subagent.clone()).0).await.unwrap();
        assert_eq!(tokio::time::timeout(std::time::Duration::from_secs(1), agent_calls.recv()).await.unwrap().unwrap().payload, subagent);
        // Waiting calls come from different hooks, so only delivery matters.
        let mut delivered = std::collections::BTreeSet::new();
        for _ in 0..20 {
            delivered.insert(bridge.calls.recv().await.unwrap().payload["text"].as_str().unwrap().parse::<u32>().unwrap());
        }
        assert_eq!(delivered, (0..20).collect());
        drop(waiting);
    }
    #[tokio::test]
    async fn claude_input_receipts_require_native_identity_and_interruption_is_execution_scoped() {
        let controls=ControlSignals::default();
        let options=Arc::new(std::sync::RwLock::new(json!({"active":true,"inputId":"run"})));
        controls.route(&json!({"type":"turn_interrupted","runId":"run","id":"execution-1"}),&options);
        controls.wait("turn_interrupted","execution-1",None).await;
        assert!(tokio::time::timeout(std::time::Duration::from_millis(10),controls.wait("turn_interrupted","execution-2",None)).await.is_err());
        controls.route(&json!({"type":"input_consumed","runId":"run","id":"bad"}),&options);
        assert!(controls.input_receipt("bad").await.is_err());
        let checkpoint=json!({"version":1,"uuid":uuid::Uuid::new_v4(),"sessionId":uuid::Uuid::new_v4()});
        controls.route(&json!({"type":"input_consumed","runId":"run","id":"good","checkpoint":checkpoint}),&options);
        assert_eq!(controls.input_receipt("good").await.unwrap(),checkpoint);
    }
    #[test]
    fn input_poll_does_not_copy_the_catalog_or_user_prompt() {
        let options = json!({"active":true,"inputId":"run","terminalText":"input","imageCount":2,"tools":["catalog"],
            "systemPrompt":"system","prompt":"user","attachmentMappings":["paths"],
            "insertion":{"inputId":"insert","terminalText":"new","imageCount":1,"prompt":"draft"}});
        assert_eq!(options_view(&options,Some("input")),json!({"active":true,"inputId":"run","terminalText":"input","imageCount":2,
            "insertion":{"inputId":"insert","terminalText":"new","imageCount":1}}));
        assert_eq!(options_view(&options,Some("session")),json!({"active":true,"attachmentMappings":["paths"]}));
        assert_eq!(options_view(&options,Some("prompt")),json!({"systemPrompt":"system"}));
        assert_eq!(options_view(&options,None),options);
        // Subagent stops ride the same poll, also while no turn is active.
        let control = json!({"commands":[{"id":"stop:a1","type":"stop","agentId":"a1"}],"stopped":["a1"]});
        let idle = json!({"active":false,"agentControl":control,"tools":["catalog"]});
        assert_eq!(options_view(&idle,Some("input")),json!({"active":false,"inputId":null,"terminalText":null,"imageCount":null,"agentControl":control}));
    }
    #[tokio::test]
    async fn image_controls_reject_stale_inputs_and_surface_preparation_errors() {
        let controls = ControlSignals::default();
        let options = Arc::new(std::sync::RwLock::new(json!({"active":true,"inputId":"run","insertion":{"inputId":"insert"}})));
        let event = json!({"type":"input_images","runId":"run","id":"insert","count":1});
        assert_eq!(controls.route(&event, &options), Some(json!({"ok":true})));
        assert!(controls.contains("input_images", "insert", Some(1)));
        assert!(!controls.contains("input_images", "insert", Some(2)));
        for bad in [json!({"runId":"old","id":"insert"}), json!({"runId":"run","id":"old"})] {
            let rejected = json!({"type":"input_images","runId":bad["runId"],"id":bad["id"],"count":2});
            assert!(controls.route(&rejected, &options).unwrap()["deny"].is_string());
        }
        let wait = controls.wait_input("input_ready", "insert", None);
        let fail = async {
            controls.route(&json!({"type":"input_staging_error","runId":"run","id":"insert","error":"Native decode failed"}), &options);
        };
        let (result, ()) = tokio::join!(wait, fail);
        assert_eq!(result, Err("Native decode failed".into()));
    }
    #[test]
    fn current_turn_activity_renews_health_without_consuming_the_event() {
        let controls = ControlSignals::default();
        let options = Arc::new(std::sync::RwLock::new(json!({"active":true,"inputId":"run"})));
        let expired = std::time::Instant::now() - super::super::bridge_health::HEARTBEAT_LEASE;
        for payload in [
            json!({"type":"thinking","runId":"run","text":""}),
            json!({"type":"phase","runId":"run","phase":"thinking"}),
            json!({"type":"checkpoint","runId":"run"}),
            json!({"tool":"Write","callId":"tool","input":{},"runId":"run"}),
        ] {
            controls.health.lock().unwrap().begin_turn("run", expired);
            assert!(controls.health_error("run").is_some());
            assert_eq!(controls.route(&payload, &options), None);
            assert!(controls.health_error("run").is_none(), "{payload}");
        }
        assert!(controls.events.lock().unwrap().is_empty());
    }

    #[test]
    fn stale_idle_unknown_and_poll_traffic_cannot_renew_health() {
        let controls = ControlSignals::default();
        let options = Arc::new(std::sync::RwLock::new(json!({"active":true,"inputId":"run"})));
        let expired = std::time::Instant::now() - super::super::bridge_health::HEARTBEAT_LEASE;
        controls.health.lock().unwrap().begin_turn("run", expired);
        for payload in [
            json!({"type":"thinking","runId":"old","text":""}),
            json!({"type":"unknown","runId":"run"}),
            json!({"type":"bridge_health","runId":"run","state":"unknown"}),
            json!({"view":"input","runId":"run"}),
            json!({"ticket":"pending","runId":"run"}),
            json!({"tool":"Write","callId":"bad","runId":"run"}),
        ] {
            controls.route(&payload, &options);
            assert!(controls.health_error("run").is_some(), "{payload}");
        }
        options.write().unwrap()["active"] = json!(false);
        controls.route(&json!({"type":"text","runId":"run","text":"late"}), &options);
        assert!(controls.health_error("run").is_some());
    }

    #[tokio::test]
    async fn worker_lifecycle_and_health_bypass_an_unfinished_tool_receipt() {
        let mut bridge = ModBridge::start().await.unwrap();
        *bridge.options.write().unwrap() = json!({"active":false});
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let ready = json!({"protocol":1,"sessionId":"native","model":"fixture"});
        let first = client.post(format!("{}/ready", bridge.url)).bearer_auth(&bridge.token).json(&ready).send();
        let accept = async {
            let call = bridge.calls.recv().await.unwrap();
            assert_eq!(call.payload["ready"], true);
            call.reply.send(json!({"ok":true})).unwrap();
        };
        let (response, ()) = tokio::join!(first, accept);
        assert_eq!(response.unwrap().json::<Value>().await.unwrap(), json!({"ok":true}));
        bridge.controls.begin_turn("run");
        *bridge.options.write().unwrap() = json!({"active":true,"inputId":"run"});
        let pending: Value = client.post(format!("{}/call", bridge.url)).bearer_auth(&bridge.token)
            .json(&json!({"protocol":1,"tool":"Write","callId":"blocked","input":{}}))
            .send().await.unwrap().json().await.unwrap();
        assert!(pending["pending"].is_string());
        let _unfinished = bridge.calls.recv().await.unwrap();
        for _ in 0..70 { // health must not evict queued-input/interruption receipts
            let response: Value = client.post(format!("{}/event", bridge.url)).bearer_auth(&bridge.token)
                .json(&json!({"protocol":1,"type":"bridge_health","runId":"run","state":"running"}))
                .send().await.unwrap().json().await.unwrap();
            assert_eq!(response, json!({"ok":true}));
        }
        assert!(bridge.controls.events.lock().unwrap().is_empty());
        assert!(bridge.controls.health_error("run").is_none());
        let restarted: Value = client.post(format!("{}/ready", bridge.url)).bearer_auth(&bridge.token).json(&ready)
            .send().await.unwrap().json().await.unwrap();
        assert!(restarted["deny"].as_str().unwrap().contains("restarted"));
        assert!(bridge.controls.health_error("run").is_some());
        assert!(bridge.calls.try_recv().is_err());
    }

    #[tokio::test]
    async fn long_tools_poll_one_submission_without_reexecution() {
        let mut bridge = ModBridge::start().await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let pending: Value = client
            .post(format!("{}/call", bridge.url))
            .bearer_auth(&bridge.token)
            .json(&json!({"protocol":1,"tool":"AskUserQuestion","callId":"ask1","input":{}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(pending["pending"].is_string());
        let call = bridge.calls.recv().await.unwrap();
        let poll = client
            .post(format!("{}/poll", bridge.url))
            .bearer_auth(&bridge.token)
            .json(&json!({"protocol":1,"ticket":pending["pending"]}))
            .send();
        let answer = async {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            call.reply
                .send(json!({"callId":"ask1","result":"answered"}))
                .unwrap();
        };
        let (result, ()) = tokio::join!(poll, answer);
        assert_eq!(
            result.unwrap().json::<Value>().await.unwrap()["result"],
            "answered"
        );
        assert!(bridge.calls.try_recv().is_err());
    }
    #[tokio::test]
    async fn bridge_authenticates_and_preserves_call_identity() {
        let mut bridge = ModBridge::start().await.unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/call", bridge.url);
        assert_eq!(
            client
                .post(&url)
                .json(&json!({}))
                .send()
                .await
                .unwrap()
                .status(),
            401
        );
        let body = json!({"protocol":1,"callId":"toolu_a","tool":"Read","input":{"file_path":"a"}});
        let request = client
            .post(&url)
            .bearer_auth(&bridge.token)
            .json(&body)
            .send();
        let handler = async {
            let call = bridge.calls.recv().await.unwrap();
            assert_eq!(call.payload, body);
            call.reply
                .send(json!({"callId":"toolu_a","deny":"not approved"}))
                .unwrap();
        };
        let (reply, ()) = tokio::join!(request, handler);
        assert_eq!(
            reply.unwrap().json::<Value>().await.unwrap()["deny"],
            "not approved"
        );
        assert_eq!(
            client
                .post(&url)
                .bearer_auth(&bridge.token)
                .header("Origin", "http://example.com")
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
}
