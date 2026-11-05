//! Process-global broker and the operations behind the C ABI. The host DLL
//! crates reach these through `export_common_ffi!` / `export_host_ffi!`;
//! non-Windows builds get inert stubs so the DLL crates still compile.

use crate::host::HostConfig;

#[cfg(windows)]
mod imp {
    use std::sync::atomic::{AtomicPtr, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    use crate::broker::Broker;
    use crate::host::HostConfig;

    /// The live broker. Swapped atomically on (re)initialization; earlier
    /// brokers are intentionally leaked so a thread still holding one never
    /// sees freed memory. The heartbeat hot path is a single atomic load.
    static CURRENT: AtomicPtr<Broker> = AtomicPtr::new(std::ptr::null_mut());

    pub(crate) fn current() -> Option<&'static Broker> {
        let ptr = CURRENT.load(Ordering::Acquire);
        // SAFETY: published pointers come from `Arc::into_raw` and are never
        // released, so they stay valid for the process lifetime.
        (!ptr.is_null()).then(|| unsafe { &*ptr })
    }

    pub fn init(config: HostConfig) -> i32 {
        if let Some(existing) = current() {
            if !existing.shutdown.load(Ordering::SeqCst) {
                // Already running (e.g. re-called after a reload). Keep the
                // live pipe and queue; the managed side re-registers.
                return 0;
            }
        }
        if config.pipe_name.trim().is_empty() {
            return -1;
        }
        // Resolve managed-runtime exports now, on the host thread, instead
        // of during a hang when the loader lock may be owned by it.
        crate::hang_stack::prepare(config.managed_namer);

        let broker = Arc::new(Broker::new(config));
        broker.publish_status_snapshot();
        let published = Arc::into_raw(Arc::clone(&broker)) as *mut Broker;
        let previous = CURRENT.swap(published, Ordering::AcqRel);
        if !previous.is_null() {
            // Lost an init race against a live broker: undo and keep it.
            // SAFETY: `previous` is a leaked `Arc::into_raw` pointer.
            let previous_live = unsafe { !(*previous).shutdown.load(Ordering::SeqCst) };
            if previous_live {
                CURRENT.store(previous, Ordering::Release);
                return 0;
            }
        }

        let thread_broker = broker;
        let spawn = std::thread::Builder::new()
            .name("locus-native-broker".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                    Ok(rt) => rt,
                    Err(e) => {
                        eprintln!("[locus-native] failed to build runtime: {e}");
                        return;
                    }
                };
                runtime.block_on(async move {
                    let observer = tokio::spawn(Arc::clone(&thread_broker).observe_main_thread());
                    crate::server::broker_main(thread_broker).await;
                    observer.abort();
                });
            });

        match spawn {
            Ok(_) => 0,
            Err(e) => {
                eprintln!("[locus-native] failed to spawn broker thread: {e}");
                -1
            }
        }
    }

    pub fn shutdown() {
        if let Some(broker) = current() {
            broker.shutdown();
        }
    }

    pub fn set_managed_state(state: i32, generation: i64, editor_status: Option<String>) {
        if let Some(broker) = current() {
            broker.set_managed_state(state, generation, editor_status);
        }
    }

    pub fn managed_heartbeat(generation: i64) {
        if let Some(broker) = current() {
            broker.heartbeat(generation);
        }
    }

    pub fn managed_execution(active: bool) {
        if let Some(broker) = current() {
            broker.execution_activity(active);
        }
    }

    pub fn set_capabilities(caps: String) {
        if let Some(broker) = current() {
            broker.set_capabilities(caps);
        }
    }

    pub fn set_host_info(json: &str) {
        if let Some(broker) = current() {
            broker.set_host_info(json);
        }
    }

    pub fn set_auth_token(token: &str) {
        if let Some(broker) = current() {
            broker.set_auth_token(token);
        }
    }

    pub fn complete_request(id: &str, response: Vec<u8>) {
        if let Some(broker) = current() {
            broker.complete(id, response);
        }
    }

    pub fn emit_event(event_type: &str, payload: String) {
        if let Some(broker) = current() {
            broker.emit_event(event_type, payload);
        }
    }

    pub fn connected() -> bool {
        current().is_some_and(|broker| broker.connected())
    }

    pub fn execution_cancel_seq() -> u64 {
        current()
            .map(|broker| broker.cancel_seq.load(Ordering::SeqCst))
            .unwrap_or(0)
    }

    pub fn execution_cancel_requested(execution_id: &str, since_seq: u64) -> bool {
        current().is_some_and(|broker| broker.execution_cancel_requested(execution_id, since_seq))
    }

    pub fn main_thread_stall_ms() -> i64 {
        current().map(|broker| broker.main_thread_stall_ms()).unwrap_or(-1)
    }

    pub fn execution_publish(execution_id: &str, json: &str) {
        if let Some(broker) = current() {
            broker.publish_execution(execution_id, json);
        }
    }

    pub fn execution_end(execution_id: &str) {
        if let Some(broker) = current() {
            broker.end_execution(execution_id);
        }
    }

    pub fn execution_client_poll_age_ms(execution_id: &str) -> i64 {
        current()
            .map(|broker| broker.execution_client_poll_age_ms(execution_id))
            .unwrap_or(-1)
    }

    /// Block until a request is queued (1), or timeout/shutdown (0), or no
    /// broker (-1). Releases nothing itself: ctypes callers drop the GIL.
    pub fn wait_request(timeout_ms: i32) -> i32 {
        let Some(broker) = current() else {
            return -1;
        };
        let timeout = Duration::from_millis(timeout_ms.max(0) as u64);
        i32::from(broker.wait_request(timeout))
    }

    /// Copy the next queued request line into `buf`. Returns the byte length
    /// written, `0` when the queue is empty, or `-1` when `buf` is too small
    /// (the request stays queued and `*out_required` holds the needed size).
    ///
    /// # Safety
    /// `buf` must be valid for `buf_len` bytes (or null with `buf_len == 0`);
    /// `out_required` must be valid or null.
    pub unsafe fn poll_request(buf: *mut u8, buf_len: i32, out_required: *mut i32) -> i32 {
        let Some(broker) = current() else {
            return 0;
        };
        let Some(needed) = broker.next_request_len() else {
            return 0;
        };
        if !out_required.is_null() {
            *out_required = needed.min(i32::MAX as usize) as i32;
        }
        if buf.is_null() || (buf_len.max(0) as usize) < needed {
            return -1;
        }
        let Some(req) = broker.take_next_request() else {
            return 0; // drained out from under us (reload/disconnect)
        };
        // The front never grows, so this only guards a concurrent swap.
        if (buf_len as usize) < req.raw.len() {
            broker.requeue_front(req);
            return -1;
        }
        std::ptr::copy_nonoverlapping(req.raw.as_ptr(), buf, req.raw.len());
        req.raw.len() as i32
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::host::HostConfig;

    pub fn init(_config: HostConfig) -> i32 {
        -1
    }
    pub fn shutdown() {}
    pub fn set_managed_state(_state: i32, _generation: i64, _editor_status: Option<String>) {}
    pub fn managed_heartbeat(_generation: i64) {}
    pub fn managed_execution(_active: bool) {}
    pub fn set_capabilities(_caps: String) {}
    pub fn set_host_info(_json: &str) {}
    pub fn set_auth_token(_token: &str) {}
    pub fn complete_request(_id: &str, _response: Vec<u8>) {}
    pub fn emit_event(_event_type: &str, _payload: String) {}
    pub fn connected() -> bool {
        false
    }
    pub fn execution_cancel_seq() -> u64 {
        0
    }
    pub fn execution_cancel_requested(_execution_id: &str, _since_seq: u64) -> bool {
        false
    }
    pub fn main_thread_stall_ms() -> i64 {
        -1
    }
    pub fn execution_publish(_execution_id: &str, _json: &str) {}
    pub fn execution_end(_execution_id: &str) {}
    pub fn execution_client_poll_age_ms(_execution_id: &str) -> i64 {
        -1
    }
    pub fn wait_request(_timeout_ms: i32) -> i32 {
        -1
    }
    /// # Safety
    /// Inert stub.
    pub unsafe fn poll_request(_buf: *mut u8, _buf_len: i32, _out_required: *mut i32) -> i32 {
        0
    }
}

pub use imp::*;


/// Start the broker for `config`. Returns 0 on success (or when a broker is
/// already running), non-zero on failure.
pub fn start(config: HostConfig) -> i32 {
    init(config)
}
