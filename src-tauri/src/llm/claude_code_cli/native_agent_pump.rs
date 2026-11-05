//! Serves a conversation's native Claude subagents independently of Locus
//! runs. Subagent callbacks arrive on their own queue (see `mod_bridge`), and
//! each one is handled in its own task: a subagent waiting for a long tool or
//! a confirmation never holds up another subagent, the parent's run, or a
//! background agent while the conversation is idle.
use super::*;
use std::sync::Weak;

pub(super) type AgentHostSlot = Arc<Mutex<Option<Arc<dyn ClaudeNativeAgentHost>>>>;

/// How long a subagent callback may precede its registration (the spawn
/// hook reports the agent only after Claude created it).
const REGISTRATION_WAIT: Duration = Duration::from_secs(10);

/// Starts the agent pump of a native-agent session. Other sessions keep every
/// callback on the run queue, exactly as before.
pub(super) fn with_agent_pump(session: Arc<Session>, locus_session_id: &str) -> Arc<Session> {
    if !session.native_agents {
        return session;
    }
    if let Ok(root) = super::super::ssh::profile_root(locus_session_id) {
        session.controls.attach_registry(root.join(AGENT_REGISTRY_FILE));
    }
    let receiver = session.bridge.try_lock().ok().and_then(|mut bridge| bridge.agent_calls.take());
    if let Some(receiver) = receiver {
        session.controls.claim_agent_pump();
        let context = PumpContext { session: Arc::downgrade(&session), session_id: locus_session_id.to_string() };
        tokio::spawn(pump(context, session.controls.clone(), session.agent_host.clone(), receiver));
    }
    session
}

#[derive(Clone)]
struct PumpContext {
    session: Weak<Session>,
    /// The Locus conversation (native task tools resolve its tasks).
    session_id: String,
}

async fn pump(
    context: PumpContext,
    controls: Arc<mod_bridge::ControlSignals>,
    slot: AgentHostSlot,
    mut calls: tokio::sync::mpsc::Receiver<mod_bridge::PendingCall>,
) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        let call = tokio::select! {
            call = calls.recv() => match call { Some(call) => call, None => break },
            _ = tick.tick() => {
                if context.session.upgrade().is_some_and(|session| session.alive()) { continue; }
                break;
            }
        };
        let Some(session) = context.session.upgrade() else {
            let _ = call.reply.send(json!({"callId":call.payload["callId"],"deny":"Claude session closed"}));
            break;
        };
        *session.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
        let host = slot.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let session_id = context.session_id.clone();
        tokio::spawn(async move {
            let reply = match host {
                Some(host) => handle(&session, &session_id, host, &call.payload).await,
                None => json!({"callId":call.payload["callId"],"deny":"Locus is not ready to run Claude subagents"}),
            };
            let _ = call.reply.send(reply);
        });
    }
    // The process is gone, and its running background agents with it.
    let agents = controls.running_background_agents();
    let host = slot.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let (false, Some(host)) = (agents.is_empty(), host) {
        host.process_exited(agents).await;
    }
}

async fn handle(session: &Arc<Session>, session_id: &str, host: Arc<dyn ClaudeNativeAgentHost>, message: &serde_json::Value) -> serde_json::Value {
    let ok = json!({"ok":true});
    match message["type"].as_str() {
        Some("agent_registered") => {
            if let Some(agent) = message["agentId"].as_str().and_then(|id| session.controls.native_agent(id)) {
                host.registered(&agent).await;
            }
            ok
        }
        Some("agent_notification") => {
            match parse_notification(message["text"].as_str().unwrap_or_default()) {
                Some(mut notice) => {
                    notice.superseded = session.controls.take_agent_notice(&notice.agent_id);
                    host.notification(&notice).await
                }
                None => eprintln!("[Claude] ignored an unrecognized task notification"),
            }
            ok
        }
        Some("control_done") => {
            if let Some(id) = message["id"].as_str() {
                session.controls.control_done(id);
                // Remove the finished command from the next input poll.
                let options = session.options.read().unwrap_or_else(|e| e.into_inner()).clone();
                let _ = session.configure(options);
            }
            ok
        }
        Some("agent_step") => {
            let id = native_agent_id(message).unwrap_or_default();
            let Some(agent) = registered(session, id).await else {
                return json!({"deny":"Unknown Claude subagent; its request was not sent."});
            };
            if session.controls.native_agent_stopped(id) {
                return json!({"deny":"The user stopped this agent."});
            }
            // A resumed run's first request can arrive before its registration.
            if !session.controls.native_agent_running(id)
                && tokio::time::timeout(REGISTRATION_WAIT, session.controls.wait_agent_running(id)).await.is_err() {
                return json!({"deny":"This agent run has ended."});
            }
            note_model(session, &host, &agent, message["model"].as_str());
            ok
        }
        Some("assistant" | "interrupted_assistant") => announce(session, &host, message).await,
        Some("tool_result") => {
            let id = message["id"].as_str().unwrap_or_default();
            if let Some(agent) = registered(session, native_agent_id(message).unwrap_or_default()).await {
                let complete = {
                    let mut tools = session.agent_tools.lock().unwrap_or_else(|e| e.into_inner());
                    tools.calls.contains_key(id) && tools.complete(id).unwrap_or(false)
                };
                // A call announced before registration has no record to close.
                if complete {
                    host.tool_finished(&agent, id, crate::tool::ToolResult {
                        output: message["output"].as_str().unwrap_or_default().into(),
                        is_error: message["isError"] == true,
                    }.into()).await;
                }
            }
            ok
        }
        Some("usage") => {
            if let Some(agent) = native_agent_id(message).and_then(|id| session.controls.native_agent(id)) {
                host.usage(&agent, &message["usage"]);
                // The request's model is preferred (agent_step); this covers a
                // step whose request did not name one.
                if session.controls.agent_model(&agent.agent_id).is_none() {
                    note_model(session, &host, &agent, message["usage"]["model"].as_str());
                }
            }
            ok
        }
        None if message["tool"].is_string() => call(session, session_id, &host, message).await,
        _ => ok,
    }
}

fn native_agent_id(message: &serde_json::Value) -> Option<&str> {
    message["agentId"].as_str().filter(|id| !id.is_empty())
}

/// The subagent's card shows the model Claude resolved for it; the host
/// hears only of a change, so a healthy agent costs one update.
fn note_model(session: &Session, host: &Arc<dyn ClaudeNativeAgentHost>, agent: &mod_bridge::NativeAgent, model: Option<&str>) {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else { return; };
    if session.controls.note_agent_model(&agent.agent_id, model) {
        host.model_resolved(agent, model);
    }
}

/// The subagent's registration, which may trail its first callbacks.
async fn registered(session: &Session, agent_id: &str) -> Option<mod_bridge::NativeAgent> {
    if agent_id.is_empty() {
        return None;
    }
    tokio::time::timeout(REGISTRATION_WAIT, session.controls.wait_native_agent(agent_id)).await.ok()
}

/// Display only: a subagent's text and calls belong to its Agent card, never
/// to the parent's conversation or assistant rounds.
async fn announce(session: &Session, host: &Arc<dyn ClaudeNativeAgentHost>, message: &serde_json::Value) -> serde_json::Value {
    let Some(agent) = registered(session, native_agent_id(message).unwrap_or_default()).await else {
        return json!({"deny":"Unknown Claude subagent"});
    };
    let blocks = message["message"]["content"].as_array().map(Vec::as_slice).unwrap_or_default();
    let mut had_tools = false;
    let mut new_tools = Vec::new();
    {
        let mut tools = session.agent_tools.lock().unwrap_or_else(|e| e.into_inner());
        for block in blocks.iter().filter(|block| block["type"] == "tool_use") {
            had_tools = true;
            match tools.announce(block) {
                Ok(true) => new_tools.push(block.clone()),
                Ok(false) => {}
                Err(error) => return json!({"deny":error}),
            }
        }
    }
    // A re-observed envelope repeats neither its text nor its calls.
    if had_tools && new_tools.is_empty() {
        return json!({"ok":true});
    }
    let joined = |kind: &str, key: &str| blocks.iter().filter(|block| block["type"] == kind)
        .filter_map(|block| block[key].as_str()).collect::<Vec<_>>().join("\n");
    let (text, thinking) = (joined("text", "text"), joined("thinking", "thinking"));
    if !text.trim().is_empty() || !thinking.trim().is_empty() {
        host.message(&agent, &thinking, &text);
    }
    if !text.trim().is_empty() {
        host.text(&agent, &text);
    }
    for block in new_tools {
        let id = block["id"].as_str().unwrap_or_default();
        let native = tool_name(block["name"].as_str().unwrap_or_default());
        let (name, arguments) = match native_tools::adapt(id, &native, &block["input"]) {
            Ok(call) => (call.locus_name.to_string(), call.locus_input),
            Err(_) => (native, block["input"].clone()),
        };
        host.tool_started(&agent, id, &name, &arguments);
    }
    json!({"ok":true})
}

fn interrupted_tool() -> ClaudeCodeToolResult {
    ClaudeCodeToolResult { output: super::super::native_journal::INTERRUPTED_DISPATCH_MESSAGE.into(), is_error: true, images: None, context: Vec::new() }
}

/// A subagent's bridged call: the same identity, journal and interruption
/// rules as the parent's calls, executed by the host's subagent channel.
async fn call(session: &Session, session_id: &str, host: &Arc<dyn ClaudeNativeAgentHost>, message: &serde_json::Value) -> serde_json::Value {
    let id = message["callId"].as_str().unwrap_or_default();
    let name = message["tool"].as_str().unwrap_or_default();
    let agent_id = native_agent_id(message).unwrap_or_default();
    let Some(agent) = registered(session, agent_id).await else {
        return json!({"callId":id,"deny":"Unknown Claude subagent; no tool was run."});
    };
    let reply = execute(session, session_id, host, &agent, message, id, name).await;
    if let Some(deny) = reply["deny"].as_str() {
        // A refusal belongs to the nested call, never to a parent round.
        host.tool_finished(&agent, id, crate::tool::ToolResult { output: deny.to_string(), is_error: true }.into()).await;
    }
    reply
}

async fn execute(
    session: &Session,
    session_id: &str,
    host: &Arc<dyn ClaudeNativeAgentHost>,
    agent: &mod_bridge::NativeAgent,
    message: &serde_json::Value,
    id: &str,
    name: &str,
) -> serde_json::Value {
    if session.controls.native_agent_stopped(&agent.agent_id) {
        return json!({"callId":id,"deny":"The user stopped this agent; no tool was run."});
    }
    {
        let mut tools = session.agent_tools.lock().unwrap_or_else(|e| e.into_inner());
        if !tools.calls.get(id).is_some_and(|call| call["name"] == name
            && native_tools::same_operation(id, name, &call["input"], &message["input"])) {
            return json!({"callId":id,"deny":"Call does not match the subagent's announced tool"});
        }
        match tools.complete(id) {
            Ok(true) => {}
            Ok(false) => return json!({"callId":id,"deny":"Duplicate tool call was not executed"}),
            Err(error) => return json!({"callId":id,"deny":error}),
        }
    }
    let Some(scope_id) = message["executionId"].as_str() else {
        return json!({"callId":id,"deny":"Missing Claude tool execution scope"});
    };
    let previous = match session.journal.begin(message) {
        Err(error) => return json!({"callId":id,"deny":error}),
        Ok(previous) => previous,
    };
    if let Some(previous) = previous {
        if previous.get("result").is_some() {
            host.tool_finished(agent, id, super::super::native_output::replay(name, &previous)).await;
        }
        return previous;
    }
    // A user stop or the subagent's own interruption ends the wait; the
    // operation itself is never repeated.
    let interrupted = async {
        tokio::select! {
            _ = session.controls.wait("turn_interrupted", scope_id, None) => {},
            _ = session.controls.wait_agent_stopped(&agent.agent_id) => {},
        }
    };
    let mut reply = if name == "SendMessage" {
        let output = tokio::select! {
            biased;
            _ = interrupted => interrupted_tool(),
            output = host.execute(agent, id, name, message["input"].clone()) => output,
        };
        match serde_json::from_str::<serde_json::Value>(&output.output) {
            Ok(result) if !output.is_error => json!({"callId":id,"result":result}),
            _ => json!({"callId":id,"deny":output.text_with_context()}),
        }
    } else if let Some(locus) = name.strip_prefix("mcp__locus__") {
        let output = tokio::select! {
            biased;
            _ = interrupted => interrupted_tool(),
            output = host.execute(agent, id, locus, message["input"].clone()) => output,
        };
        let mut reply = if output.is_error || output.output == crate::session::history::INTERRUPTED_TOOL_RESULT {
            json!({"callId":id,"deny":output.text_with_context()})
        } else {
            json!({"callId":id,"result":mcp_tool_result_content(&output)})
        };
        reply["locusOutput"] = json!(output.text_with_context());
        reply
    } else {
        match native_tools::adapt(id, &tool_name(name), &message["input"]) {
            Ok(native) => {
                let scope = native_scope::NativeInvocation::new(native.clone(), session_id.to_string());
                let output = tokio::select! {
                    biased;
                    _ = interrupted => interrupted_tool(),
                    output = scope.run(host.execute(agent, id, native.locus_name, native.locus_input.clone())) => output,
                };
                let mut reply = native_tools::finish_result(&native, &output, scope.result());
                reply["locusOutput"] = json!(output.text_with_context());
                reply
            }
            Err(error) => json!({"callId":id,"deny":error}),
        }
    };
    if let Err(error) = session.journal.complete(id, &reply) {
        reply = json!({"callId":id,"deny":format!("Failed to persist local tool outcome: {error}. The operation will not be automatically repeated.")});
    }
    reply
}

/// Stops a native subagent for the user: a persisted stop flag (no
/// automatic resume, also after a restart), then Claude's own TaskStop
/// through the Mod's input poll.
pub(crate) fn stop_native_agent(session_id: &str, agent_id: &str) -> Result<(), String> {
    let session = sessions().lock().unwrap_or_else(|e| e.into_inner()).get(session_id).cloned();
    let Some(session) = session.filter(|session| session.native_agents) else {
        // No process, so nothing runs; keep the flag for the next one.
        let controls = mod_bridge::ControlSignals::default();
        controls.attach_registry(super::super::ssh::profile_root(session_id)?.join(AGENT_REGISTRY_FILE));
        controls.stop_native_agent(agent_id);
        return Ok(());
    };
    session.controls.request_agent_stop(agent_id);
    let options = session.options.read().unwrap_or_else(|e| e.into_inner()).clone();
    session.configure(options)
}

pub(super) const AGENT_REGISTRY_FILE: &str = "native_agents.json";

/// The conversation's native Agent cards (subagent and model), for its
/// history. The registry is current while Claude runs too.
pub(crate) fn native_agent_cards(session_id: &str) -> Result<Vec<mod_bridge::RegistryCard>, String> {
    let path = super::super::ssh::profile_root(session_id)?.join(AGENT_REGISTRY_FILE);
    Ok(mod_bridge::registry_cards(&path))
}

/// `<task-notification>` as Claude 2.1.312 renders it for an agent.
pub(super) fn parse_notification(text: &str) -> Option<ClaudeCodeAgentNotice> {
    let body = text.trim().strip_prefix("<task-notification>")?.strip_suffix("</task-notification>")?;
    let tag = |name: &str| {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let start = body.find(&open)? + open.len();
        let end = body[start..].find(&close)? + start;
        Some(body[start..end].to_string())
    };
    let agent_id = tag("task-id").filter(|id| !id.trim().is_empty())?;
    Some(ClaudeCodeAgentNotice {
        agent_id: agent_id.trim().to_string(),
        tool_use_id: tag("tool-use-id").unwrap_or_default().trim().to_string(),
        status: tag("status")?.trim().to_string(),
        summary: tag("summary").unwrap_or_default(),
        result: tag("result"),
        superseded: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_notifications_of_completed_and_stopped_agents_parse() {
        let completed = "<task-notification>\n<task-id>ae1391f24061f2eb0</task-id>\n<tool-use-id>toolu_native_1</tool-use-id>\n<status>completed</status>\n<summary>Agent \"Native fixture 1\" finished</summary>\n<note>A task-notification fires each time this agent stops.</note>\n<result>NATIVE_DONE\n<b>tags stay</b></result>\n<usage><subagent_tokens>1104</subagent_tokens></usage>\n</task-notification>";
        assert_eq!(parse_notification(completed), Some(ClaudeCodeAgentNotice {
            agent_id: "ae1391f24061f2eb0".into(), tool_use_id: "toolu_native_1".into(), status: "completed".into(),
            summary: "Agent \"Native fixture 1\" finished".into(), result: Some("NATIVE_DONE\n<b>tags stay</b>".into()),
            superseded: false,
        }));
        let killed = "<task-notification>\n<task-id>a92abd40066ed59f2</task-id>\n<tool-use-id>toolu_native_1</tool-use-id>\n<status>killed</status>\n<summary>Agent \"Native fixture 1\" was stopped by Claude</summary>\n</task-notification>";
        let notice = parse_notification(killed).unwrap();
        assert_eq!((notice.status.as_str(), notice.result), ("killed", None));
        for invalid in ["", "<task-notification></task-notification>", "<task-notification><status>completed</status></task-notification>", "plain text"] {
            assert!(parse_notification(invalid).is_none(), "{invalid}");
        }
    }
}
