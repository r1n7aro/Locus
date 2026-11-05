//! Bounded native-file RPC carried on the existing SSH stdio connection.
use super::*;
use std::collections::HashMap;

const LIMIT: usize = 256 * 1024 * 1024;
struct Pending {
    sender: oneshot::Sender<Result<Value, String>>,
    chunks: serde_json::Map<String, Value>,
    size: usize,
    progress: Arc<transfer::Progress>,
}
#[derive(Default)]
pub(super) struct ArchiveRpc(Mutex<HashMap<String, Pending>>);
struct CancelOnDrop {rpc:Arc<ArchiveRpc>,id:String}
impl Drop for CancelOnDrop {fn drop(&mut self){self.rpc.cancel(&self.id);}}
impl ArchiveRpc {
    pub fn busy(&self) -> bool {
        !self.0.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
    }
    pub fn register(&self, id: &str) -> Result<(oneshot::Receiver<Result<Value, String>>, Arc<transfer::Progress>), String> {
        let mut requests=self.0.lock().map_err(|e|e.to_string())?;
        if !requests.is_empty() { return Err("Native archive synchronization is already running".into()); }
        let (sender,receiver)=oneshot::channel();
        let progress=Arc::new(transfer::Progress::default());
        requests.insert(id.into(),Pending{sender,chunks:serde_json::Map::new(),size:0,progress:progress.clone()});
        Ok((receiver,progress))
    }
    pub fn frame_progress(&self, value:&Value)->Result<Option<transfer::FrameProgress>,String> {
        if value["type"]!="progress" || value["protocol"]!=1 || value["stage"]!="archive_transfer" {return Ok(None);}
        let requests=self.0.lock().unwrap_or_else(|e|e.into_inner());
        let Some((id,request))=value["id"].as_str().and_then(|id|requests.get(id).map(|request|(id,request))) else {return Ok(None);};
        transfer::FrameProgress::new(request.progress.clone(),transfer::announced_size(value)?,Some(id.into())).map(Some)
    }
    pub fn route(&self, value: &Value) -> bool {
        if !matches!(value["type"].as_str(),Some("archive_chunk"|"archive_result"|"archive_error")) { return false; }
        let Some(id)=value["id"].as_str() else { return true; };
        let mut requests=self.0.lock().unwrap_or_else(|e|e.into_inner());
        if value["type"]=="archive_chunk" {
            let Some(request)=requests.get_mut(id) else { return true; };
            let data=value["data"].as_str().unwrap_or_default();
            let name=value["name"].as_str().unwrap_or_default();
            request.size=request.size.saturating_add(data.len());
            if data.len()>512*1024 || request.size>LIMIT*4/3+4 || !super::valid_name(name) {
                if let Some(request)=requests.remove(id) { let _=request.sender.send(Err("Invalid or oversized native archive stream".into())); }
                return true;
            }
            // Legacy helpers have no size markers; their completed, matching
            // chunks still renew progress. Pongs/terminal/other RPC IDs never do.
            if !data.is_empty() {request.progress.received();}
            request.chunks.entry(name).or_insert_with(||json!([])).as_array_mut().unwrap().push(json!(data));
        } else if let Some(request)=requests.remove(id) {
            let result=if value["type"]=="archive_error" {
                Err(value["message"].as_str().unwrap_or("Native archive failed").to_string())
            } else {
                let mut result=value["value"].clone();
                if !result.is_object() { Err("Invalid native archive response".into()) }
                else { result["chunks"]=Value::Object(request.chunks); Ok(result) }
            };
            let _=request.sender.send(result);
        }
        true
    }
    pub fn cancel(&self,id:&str) { self.0.lock().unwrap_or_else(|e|e.into_inner()).remove(id); }
    pub fn close(&self, error: &str) {
        for (_,request) in self.0.lock().unwrap_or_else(|e|e.into_inner()).drain() {
            let _=request.sender.send(Err(error.to_string()));
        }
    }
}

impl Transport {
    pub async fn archive_request(&self, mut request:Value)->Result<Value,String> {
        let id=uuid::Uuid::new_v4().to_string();
        let (receiver,progress)=self.archive_rpc.register(&id)?;
        let _cancel=CancelOnDrop{rpc:self.archive_rpc.clone(),id:id.clone()};
        if request.get("type").is_none(){request["type"]=json!("archive");}
        request["id"]=json!(id);
        if !self.alive(){return Err(self.failure());}
        // A large restore can fill the bounded writer queue. Preserve chunk /
        // commit ordering and wait for capacity instead of losing the commit.
        self.send.send(request).await.map_err(|_|self.failure())?;
        let response=progress.wait(receiver,transfer::Limits::archive()).await;
        self.archive_rpc.cancel(&id);
        response.map_err(|timeout|super::errors::disconnected(timeout.message("Native archive synchronization")))?
            .map_err(|_|self.failure())?
    }
    pub async fn archive_upload_chunk(&self,value:Value)->Result<(),String> {
        if !self.alive(){return Err(self.failure());}
        self.send.send(value).await.map_err(|_|self.failure())
    }
}
