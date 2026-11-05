//! Original native records are separate from the display transcript. Only
//! verified Claude archives can be restored; API history is never converted.
use serde::{Deserialize,Serialize};
use super::connection::NativeConnection;
use std::collections::BTreeMap;
use sha2::{Digest,Sha256};
use base64::Engine;
use locus_remote_core::claude_ssh::archive_pool::{self, Priority};

fn retention() -> std::time::Duration {
    let minutes = crate::settings::global::global().get(crate::settings::PROVIDERS_CLAUDE_CODE_IDLE_MINUTES);
    std::time::Duration::from_secs(u64::from(minutes.clamp(1, 60)) * 60)
}
fn background_enabled() -> bool {
    crate::settings::global::global().get(crate::settings::PROVIDERS_CLAUDE_CODE_ENABLED)
}
pub(crate) fn prewarm(remote: &super::ssh::SshConfig) {
    if background_enabled() { archive_pool::prewarm(remote, retention()); }
}
mod projection;
pub use projection::project;

pub const LIMIT: usize=256*1024*1024;
#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(rename_all="camelCase")]
pub struct Cursor { pub size:u64,pub sha256:String }
pub type Manifest=BTreeMap<String,Cursor>;
#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub struct ArchiveInfo {
    pub version:u32,
    pub native_id:String,
    pub source_root:String,
    pub manifest:Manifest,
    pub updated_at:i64,
    pub transferred_bytes:u64,
    #[serde(default)] pub incomplete_tail:bool,
}
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct Change {pub name:String,pub offset:u64,pub base_hash:Option<String>}
#[derive(Deserialize)]
#[serde(rename_all="camelCase")]
pub struct Delta {
    pub version:u32,pub native_id:String,pub source_root:String,
    pub manifest:Manifest,pub changes:Vec<Change>,pub transferred_bytes:u64,
    #[serde(default)] pub incomplete_tail:bool,
    pub chunks:BTreeMap<String,Vec<String>>,
}
pub use locus_remote_core::claude_ssh::valid_name;
pub fn digest(bytes:&[u8])->String {Sha256::digest(bytes).iter().map(|byte|format!("{byte:02x}")).collect()}
impl Delta {
    pub fn apply(&self, old:&BTreeMap<String,Vec<u8>>, expected_id:&str)->Result<BTreeMap<String,Vec<u8>>,String> {
        if self.version!=1 || self.native_id!=expected_id || uuid::Uuid::parse_str(expected_id).is_err() || self.manifest.len()>10000
            || !self.manifest.contains_key("transcript.jsonl") {return Err("Invalid native archive identity or manifest".into());}
        let mut files=old.clone();
        files.retain(|name,_|self.manifest.contains_key(name));
        let mut changed=std::collections::HashSet::new();
        let mut transferred=0usize;
        for change in &self.changes {
            if !valid_name(&change.name) || !changed.insert(&change.name) || !self.manifest.contains_key(&change.name) {return Err("Invalid native archive patch".into());}
            let data=files.entry(change.name.clone()).or_default();
            if change.offset==0 {data.clear();}
            else if change.name!="transcript.jsonl" || change.offset!=data.len() as u64 || change.base_hash.as_deref()!=Some(digest(data).as_str()) {
                return Err("Native archive delta does not match the saved prefix".into());
            }
            for chunk in self.chunks.get(&change.name).into_iter().flatten() {
                let bytes=base64::engine::general_purpose::STANDARD.decode(chunk).map_err(|_|"Invalid native archive encoding")?;
                transferred=transferred.checked_add(bytes.len()).ok_or("Native archive size overflow")?;
                if transferred>LIMIT || data.len()+bytes.len()>LIMIT {return Err("Native archive exceeds 256 MiB".into());}
                data.extend(bytes);
            }
        }
        if self.chunks.keys().any(|name|!changed.contains(name)) || transferred as u64!=self.transferred_bytes {return Err("Unexpected native archive chunks".into());}
        let mut total=0usize;
        for (name,cursor) in &self.manifest {
            let bytes=files.get(name).ok_or("Native archive file is missing")?;
            total=total.checked_add(bytes.len()).ok_or("Native archive size overflow")?;
            if !valid_name(name) || total>LIMIT || bytes.len() as u64!=cursor.size || digest(bytes)!=cursor.sha256 {return Err("Native archive checksum mismatch".into());}
        }
        let transcript=&files["transcript.jsonl"];
        if !transcript.is_empty()&&!transcript.ends_with(b"\n") {return Err("Native archive ends in an incomplete JSONL record".into());}
        Ok(files)
    }
}

pub async fn download(id:&str,remote:&NativeConnection,native_id:&str,cursor:&Manifest,checkpoint:Option<&str>)->Result<Delta,String> {
    download_with_priority(id,remote,native_id,cursor,checkpoint,Priority::Foreground).await
}
async fn download_with_priority(id:&str,remote:&NativeConnection,native_id:&str,cursor:&Manifest,checkpoint:Option<&str>,priority:Priority)->Result<Delta,String> {
    let mut request=serde_json::json!({"nativeId":native_id,"cursor":cursor,"checkpoint":checkpoint});
    let value=match remote {
        NativeConnection::Ssh(remote) => {
            request["sessionKey"]=serde_json::json!(super::ssh::session_key(id,remote));
            archive_pool::download(remote,request,priority,retention()).await?
        }
        NativeConnection::Server {binding_id} => server_operation(binding_id,id,"archive",request).await?,
    };
    let delta=serde_json::from_value::<Delta>(value).map_err(|e|format!("Invalid native archive delta: {e}"))?;
    if delta.native_id!=native_id {return Err("Native archive returned another conversation identity".into());}
    Ok(delta)
}

fn gate(id:&str)->std::sync::Arc<tokio::sync::Mutex<()>> {
    use std::sync::{Arc,Mutex,OnceLock,Weak};
    static GATES:OnceLock<Mutex<BTreeMap<String,Weak<tokio::sync::Mutex<()>>>>>=OnceLock::new();
    let mut gates=GATES.get_or_init(Default::default).lock().unwrap_or_else(|e|e.into_inner());
    if let Some(gate)=gates.get(id).and_then(Weak::upgrade) {return gate;}
    gates.retain(|_,g|g.strong_count()>0);
    let gate=Arc::new(tokio::sync::Mutex::new(()));gates.insert(id.into(),Arc::downgrade(&gate));gate
}
pub async fn backup(store:&crate::session::store::SessionStore,id:&str)->Result<Option<ArchiveInfo>,String> {
    backup_inner(store,id,None,true,Priority::Foreground).await
}
pub(super) async fn backup_for_history(store:&crate::session::store::SessionStore,id:&str,pending:Option<&str>)->Result<Option<ArchiveInfo>,String> {
    // A Mod append acknowledgement can precede the native JSONL flush. After
    // a crash that checkpoint may never exist. Recovery reads the committed
    // source and its dispatch receipts; only live backups wait for a checkpoint.
    backup_inner(store,id,pending,false,Priority::Foreground).await
}
/// Refresh the local archive before Claude compacts. The rows being compacted
/// are already in native history, so this never waits for a new checkpoint.
pub(crate) async fn refresh_for_compaction(store:&crate::session::store::SessionStore,id:&str)->Result<Option<ArchiveInfo>,String> {
    backup_inner(store,id,None,false,Priority::Foreground).await
}
async fn backup_inner(store:&crate::session::store::SessionStore,id:&str,pending:Option<&str>,require_checkpoint:bool,priority:Priority)->Result<Option<ArchiveInfo>,String> {
    let _guard=gate(id).lock_owned().await;
    let Some(remote)=super::recovery::recorded_remote(store,id)? else {return Ok(None);};
    let Some(native_id)=super::recovery::native_session_id(store,id)? else {return Ok(None);};
    let checkpoint=if require_checkpoint {
        let messages=store.get_messages(id)?;let metadata=store.get_response_request_metadata(id)?;
        let pending_ids=store.claude_recovery_pending_messages(id,&messages,&metadata,pending)?;
        store.verify_claude_history_origin(id,&messages,&metadata,&pending_ids)?;
        messages.iter().rev().find_map(|message|metadata.get(&message.id).and_then(|value|value["claude_checkpoint"]["uuid"].as_str()).map(str::to_owned))
    } else {None};
    let cursor=store.claude_archive_info(id)?.filter(|info|info.native_id==native_id).map(|info|info.manifest).unwrap_or_default();
    let mut attempt=0;
    let delta=loop {
        match download_with_priority(id,&remote,&native_id,&cursor,checkpoint.as_deref(),priority).await {
            Ok(delta)=>break delta,
            Err(error) if error.contains("not flushed") && attempt<3=>{attempt+=1;tokio::time::sleep(std::time::Duration::from_millis(250)).await;},
            Err(error)=>return Err(error),
        }
    };
    let store=store.clone();let id=id.to_string();
    tokio::task::spawn_blocking(move||store.save_claude_archive(&id,&remote,&delta,&cursor)).await.map_err(|e|e.to_string())?.map(Some)
}
#[derive(Default)]
struct ScheduledBackups(std::collections::HashMap<String,bool>);
impl ScheduledBackups {
    /// True only for the first event: later ones update the same pending job.
    fn mark(&mut self,id:&str)->bool { self.0.insert(id.into(),true).is_none() }
    fn begin(&mut self,id:&str) { if let Some(dirty)=self.0.get_mut(id) { *dirty=false; } }
    fn finish(&mut self,id:&str)->bool {
        if self.0.get(id).copied().unwrap_or(false) { true }
        else { self.0.remove(id); false }
    }
}
fn scheduled()->&'static std::sync::Mutex<ScheduledBackups> {
    static PENDING:std::sync::OnceLock<std::sync::Mutex<ScheduledBackups>>=std::sync::OnceLock::new();
    PENDING.get_or_init(Default::default)
}

/// Event-driven coalescing: one running backup plus a dirty bit per session.
/// Checkpoints arriving DURING a transfer must request a subsequent refresh.
/// Final turn completion schedules here too; only explicit export/recovery and
/// pre-compaction reads wait for an up-to-date archive.
pub fn schedule(store:crate::session::store::SessionStore,id:String,app:tauri::AppHandle) {
    if !background_enabled() { return; }
    let pending=scheduled();
    if !pending.lock().unwrap_or_else(|e|e.into_inner()).mark(&id) { return; }
    tokio::spawn(async move {
        let mut delay=std::time::Duration::from_secs(3);
        loop {
            tokio::time::sleep(delay).await;
            if !background_enabled() {
                pending.lock().unwrap_or_else(|e|e.into_inner()).0.remove(&id);
                break;
            }
            pending.lock().unwrap_or_else(|e|e.into_inner()).begin(&id);
            let result=backup_inner(&store,&id,None,true,Priority::Background).await;
            if let Err(error)=&result {
                crate::error::AppError::emit_background(&app,&crate::error::AppError::new(
                    "claude.archive_failed","Claude session backup was not updated; the previous backup was retained.")
                    .detail(error.clone()).operation("claudeArchive").severity(crate::error::ErrorSeverity::Warning));
            }
            delay=std::time::Duration::from_secs(if result.is_err() {60} else {3});
            if !pending.lock().unwrap_or_else(|e|e.into_inner()).finish(&id) { break; }
        }
    });
}

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct RestoreReport {pub native_id:String,pub target:String,pub transferred_bytes:u64}
#[derive(Clone,Serialize)]
#[serde(rename_all="camelCase")]
pub struct TransferCheck {
    pub source:String,pub target:String,pub target_fingerprint:String,pub native_id:String,
    pub archive_digest:Option<String>,pub updated_at:Option<i64>,pub blocked_reason:Option<String>,
}
fn archive_digest(info:&ArchiveInfo)->String {
    digest(&serde_json::to_vec(&(&info.native_id,&info.manifest,info.incomplete_tail)).expect("archive manifest serialization"))
}
/// Local-only preflight. Opening the reminder must not connect or deploy.
pub fn transfer_check(store:&crate::session::store::SessionStore,id:&str)->Result<Option<TransferCheck>,String> {
    let Some(source)=super::recovery::recorded_remote(store,id)? else {return Ok(None);};
    let settings=super::ssh::load_settings()?;
    let Some(target)=settings.remote else {return Ok(None);};
    let Some(source) = source.as_ssh() else { return Ok(None); };
    if source==&target {return Ok(None);}
    let Some(native_id)=super::recovery::native_session_id(store,id)? else {return Ok(None);};
    let info=store.claude_archive_info(id)?.filter(|info|info.native_id==native_id);
    let blocked_reason=if store.active_run_for_session(id)?.is_some(){Some("running")}
        else if settings.authorized_target.as_deref()!=Some(target.fingerprint().as_str()){Some("unauthorized")}
        else if info.is_none(){Some("noBackup")}
        else if info.as_ref().is_some_and(|info|info.incomplete_tail){Some("incomplete")}
        else {None};
    Ok(Some(TransferCheck{source:source.target.clone(),target:target.target.clone(),target_fingerprint:target.fingerprint(),native_id,
        archive_digest:info.as_ref().map(archive_digest),updated_at:info.as_ref().map(|i|i.updated_at),blocked_reason:blocked_reason.map(str::to_string)}))
}
pub async fn restore(store:&crate::session::store::SessionStore,id:&str,expected_target:&str,expected_native:&str,expected_archive:&str)->Result<RestoreReport,String> {
    let _history=super::tui::history_gate(id).try_write_owned().map_err(|_|"Stop the conversation before restoring it on another server")?;
    if store.active_run_for_session(id)?.is_some(){return Err("Stop the conversation before restoring it on another server".into());}
    // A finished chat can still have its final backup queued. Flush that demand
    // before using the restore snapshot; do not silently move an older turn.
    // With no pending work, preserve the existing offline-restore behavior.
    let pending=scheduled().lock().unwrap_or_else(|e|e.into_inner()).0.contains_key(id);
    if pending { backup(store,id).await?; }
    let _archive=gate(id).lock_owned().await;
    store.verify_claude_history_origin(id,&store.get_messages(id)?,&store.get_response_request_metadata(id)?,&[])?;
    let settings=super::ssh::load_settings()?;
    let recorded=super::recovery::recorded_remote(store,id)?;
    let remote=match recorded {
        Some(remote @ NativeConnection::Server{..}) if remote.fingerprint()==expected_target => remote,
        _ => {
            let ssh=settings.remote.ok_or("Select and authorize the destination SSH connection in Settings")?;
            if settings.authorized_target.as_deref()!=Some(ssh.fingerprint().as_str()){return Err("Authorize the destination server first".into());}
            NativeConnection::Ssh(ssh)
        }
    };
    let info=store.claude_archive_info(id)?.ok_or("This conversation has no saved native Claude archive")?;
    if remote.fingerprint()!=expected_target || info.native_id!=expected_native || archive_digest(&info)!=expected_archive {
        return Err("The destination or local backup changed. Review the send reminder again.".into());
    }
    if super::recovery::native_session_id(store,id)?.as_deref()!=Some(info.native_id.as_str()){return Err("The native archive does not match the current conversation identity".into());}
    let files=store.claude_archive_files(id)?;
    let snapshot=project(&info,&files)?;
    snapshot.can_continue()?;
    let confirmed=verify_restore_coverage(&snapshot,&files,&store.get_messages_for_prompt(id)?)?;
    super::native_journal::Journal::new(super::ssh::profile_root(id)?.join("calls")).verify_archive(&confirmed,info.updated_at)?;
    let messages=store.get_messages(id)?;
    let expected=store.claude_endpoint_manifest(id,&remote,&info.native_id)?;
    let mut prefix_hashes=BTreeMap::new();
    if let (Some(old),Some(data))=(expected.get("transcript.jsonl"),files.get("transcript.jsonl")) {
        if old.size<=data.len() as u64 {prefix_hashes.insert("transcript.jsonl",digest(&data[..old.size as usize]));}
    }
    super::tui::close_session(id);
    let transport=ArchiveTransport::start(id,&remote).await?;
    let prepared=transport.archive_request(serde_json::json!({"type":"restore_begin","nativeId":info.native_id,
        "manifest":info.manifest,"expected":expected,"prefixHashes":prefix_hashes})).await?;
    let upload=prepared["uploadId"].as_str().ok_or("Missing native archive upload identity")?;
    uuid::Uuid::parse_str(upload).map_err(|_|"Invalid native archive upload identity")?;
    let mut names=std::collections::HashSet::new();let mut transferred=0u64;
    for needed in prepared["needs"].as_array().ok_or("Invalid native restore request")? {
        let name=needed["name"].as_str().ok_or("Invalid native restore filename")?;
        let data=files.get(name).ok_or("Server requested a file outside the native archive")?;
        let offset=needed["offset"].as_u64().ok_or("Invalid native restore offset")?;
        if !names.insert(name) || offset>data.len() as u64 || (offset>0&&(name!="transcript.jsonl" || expected.get(name).is_none_or(|old|old.size!=offset||digest(&data[..offset as usize])!=old.sha256))) {
            return Err("Server requested an invalid native archive range".into());
        }
        for chunk in data[offset as usize..].chunks(192*1024) {
            transport.archive_upload_chunk(serde_json::json!({"type":"restore_chunk","uploadId":upload,"name":name,
                "data":base64::engine::general_purpose::STANDARD.encode(chunk)})).await?;
            transferred+=chunk.len() as u64;
        }
    }
    let result=transport.archive_request(serde_json::json!({"type":"restore_commit","uploadId":upload})).await?;
    if result["nativeId"]!=info.native_id || result["transferredBytes"].as_u64()!=Some(transferred)
        || serde_json::from_value::<Manifest>(result["manifest"].clone()).map_err(|_|"Invalid native restore manifest")?!=info.manifest {
        return Err("Server did not confirm the expected native archive".into());
    }
    // No model call has happened. Only after the server confirms every byte
    // may subsequent TUI turns use the destination's own authentication.
    store.restore_claude_native_binding(id,&remote,&snapshot,&messages)?;
    if let Some(remote)=remote.as_ssh() {super::ssh::bind_restored_connection(id,remote)?;}
    super::store_session_id(id,&info.native_id).await;
    Ok(RestoreReport{native_id:info.native_id,target:remote.as_ssh().map(|ssh|ssh.target.clone()).unwrap_or_else(||"Locus Server".into()),transferred_bytes:transferred})
}

fn verify_restore_coverage(snapshot:&super::recovery::NativeSnapshot,files:&BTreeMap<String,Vec<u8>>,messages:&[crate::session::models::ChatMessage])->Result<std::collections::HashSet<String>,String> {
    let mut confirmed:std::collections::HashSet<String>=snapshot.messages.iter().filter_map(|m|m.tool_call_id.clone()).collect();
    for (name,data) in files.iter().filter(|(name,_)|name.starts_with("dispatch/")) {
        let record:serde_json::Value=serde_json::from_slice(data).map_err(|_|format!("Invalid native receipt: {name}"))?;
        if record["version"]==2 && record["settled"]==true {
            if let Some(call)=record["callId"].as_str(){confirmed.insert(call.into());}
        }
    }
    if messages.iter().any(|m|m.tool_calls.iter().flatten().any(|call|!confirmed.contains(&call.id))
        || m.tool_call_id.as_ref().is_some_and(|id|!confirmed.contains(id))) {
        return Err("The local backup does not confirm all recorded tool operations. Synchronize the source server before moving this conversation; tools were not repeated.".into());
    }
    Ok(confirmed)
}

async fn server_operation(binding:&str,id:&str,operation:&str,request:serde_json::Value)->Result<serde_json::Value,String>{
    #[cfg(feature = "server-client")]
    {super::server_transport::operation(binding,id,operation,request).await}
    #[cfg(not(feature = "server-client"))]
    {let _=(binding,id,operation,request);Err("This conversation requires the internal Locus Server client".into())}
}
#[cfg(test)]
mod scheduling_tests {
    use super::ScheduledBackups;
    #[test]
    fn checkpoints_during_a_transfer_get_one_follow_up_and_never_get_lost() {
        let mut pending=ScheduledBackups::default();
        assert!(pending.mark("a"));
        assert!(!pending.mark("a"));
        pending.begin("a");
        for _ in 0..100 { assert!(!pending.mark("a")); }
        assert!(pending.finish("a"));
        pending.begin("a");
        assert!(!pending.finish("a"));
        assert!(pending.0.is_empty());
        assert!(pending.mark("a"));
    }
    #[test]
    fn independent_sessions_do_not_extend_each_others_work() {
        let mut pending=ScheduledBackups::default();
        pending.mark("a"); pending.mark("b"); pending.begin("a");
        assert!(!pending.finish("a"));
        assert!(pending.finish("b"));
        assert!(!pending.0.contains_key("a"));
    }
}

enum ArchiveTransport {Ssh(super::ssh::Transport),Server{binding:String,id:String}}
impl ArchiveTransport {
    async fn start(id:&str,remote:&NativeConnection)->Result<Self,String>{
        match remote {
            NativeConnection::Ssh(remote)=>{
                let (transport,_)=super::ssh::Transport::start(remote,serde_json::json!({"mode":"archive","authorizedSetup":true,"sessionKey":super::ssh::session_key(id,remote)}),None).await?;
                Ok(Self::Ssh(transport))
            }
            NativeConnection::Server{binding_id}=>Ok(Self::Server{binding:binding_id.clone(),id:id.into()}),
        }
    }
    async fn archive_request(&self,mut value:serde_json::Value)->Result<serde_json::Value,String>{
        match self {
            Self::Ssh(transport)=>transport.archive_request(value).await,
            Self::Server{binding,id}=>{
                let operation=match value["type"].as_str(){Some("restore_begin")=>"restore-begin",Some("restore_commit")=>"restore-commit",_=>return Err("Unsupported archive operation".into())};
                if let Some(value)=value.as_object_mut(){value.remove("type");}
                server_operation(binding,id,operation,value).await
            }
        }
    }
    async fn archive_upload_chunk(&self,mut value:serde_json::Value)->Result<(),String>{
        match self {
            Self::Ssh(transport)=>transport.archive_upload_chunk(value).await,
            Self::Server{binding,id}=>{
                if let Some(value)=value.as_object_mut(){value.remove("type");}
                server_operation(binding,id,"restore-chunk",value).await.map(|_|())
            }
        }
    }
}
