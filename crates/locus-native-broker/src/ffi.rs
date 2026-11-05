//! C ABI generation. The broker crate exports no symbols itself; each host
//! DLL invokes these macros once, so the common entry points have the same
//! names and signatures in every DLL and nothing leaks across hosts.

/// # Safety
/// `ptr` must be valid for `len` bytes or null. Returns an empty slice for
/// null / non-positive length.
#[doc(hidden)]
pub unsafe fn slice_from_raw<'a>(ptr: *const u8, len: i32) -> &'a [u8] {
    if ptr.is_null() || len <= 0 {
        &[]
    } else {
        std::slice::from_raw_parts(ptr, len as usize)
    }
}

/// # Safety
/// See [`slice_from_raw`]. Decodes loss-tolerantly; the broker never trusts
/// the bytes to be valid UTF-8.
#[doc(hidden)]
pub unsafe fn string_from_raw(ptr: *const u8, len: i32) -> String {
    String::from_utf8_lossy(slice_from_raw(ptr, len)).into_owned()
}

/// The sixteen entry points every host uses (the original `locus_native`
/// surface). `$config` is `fn(project: String, pipe: String, protocol: i32)
/// -> Option<HostConfig>` building the host configuration for `locus_init`.
#[macro_export]
macro_rules! export_common_ffi {
    ($config:path) => {
        /// Initialize the broker and start the pipe server. Idempotent: a
        /// second call (e.g. after a reload) keeps the live pipe. Returns 0
        /// on success, non-zero on failure.
        ///
        /// # Safety
        /// `project`/`pipe` must each be valid for their stated lengths or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_init(
            project: *const u8,
            project_len: i32,
            pipe: *const u8,
            pipe_len: i32,
            protocol_version: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let project = $crate::ffi::string_from_raw(project, project_len);
            let pipe = $crate::ffi::string_from_raw(pipe, pipe_len);
            if pipe.trim().is_empty() {
                return -1;
            }
            match $config(project, pipe, protocol_version) {
                Some(config) => $crate::api::init(config),
                None => -1,
            }
        }

        /// Stop the pipe server. Not called on a managed reload (the broker
        /// must persist) — only when the host quits or unloads the DLL.
        #[no_mangle]
        pub extern "C" fn locus_shutdown() {
            $crate::api::shutdown();
        }

        /// Register the managed lifecycle state (`managed_state::*`),
        /// generation (ignored if `<= 0`) and the cached status string used to
        /// answer `status` during a reload.
        ///
        /// # Safety
        /// `editor_status` must be valid for `editor_status_len` bytes or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_set_managed_state(
            state: ::std::os::raw::c_int,
            generation: i64,
            editor_status: *const u8,
            editor_status_len: i32,
        ) {
            let status = if editor_status.is_null() {
                None
            } else {
                Some($crate::ffi::string_from_raw(editor_status, editor_status_len))
            };
            $crate::api::set_managed_state(state, generation, status);
        }

        /// Pulse the main-thread heartbeat (one atomic increment).
        #[no_mangle]
        pub extern "C" fn locus_managed_heartbeat(generation: i64) {
            $crate::api::managed_heartbeat(generation);
        }

        /// Execution lifetime is independent of the request's response/timeout.
        #[no_mangle]
        pub extern "C" fn locus_managed_execution(active: ::std::os::raw::c_int) {
            $crate::api::managed_execution(active != 0);
        }

        /// Register the managed executor's capability string.
        ///
        /// # Safety
        /// `caps` must be valid for `caps_len` bytes or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_set_capabilities(caps: *const u8, caps_len: i32) {
            $crate::api::set_capabilities($crate::ffi::string_from_raw(caps, caps_len));
        }

        /// Dequeue the next request line for the managed executor: byte
        /// length, `0` when empty, `-1` when the buffer is too small.
        ///
        /// # Safety
        /// `buffer` must be valid for `buffer_len` bytes (or null when
        /// `buffer_len` is 0); `out_required_len` must be valid or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_poll_request(
            buffer: *mut u8,
            buffer_len: i32,
            out_required_len: *mut i32,
        ) -> ::std::os::raw::c_int {
            $crate::api::poll_request(buffer, buffer_len, out_required_len)
        }

        /// Hand a completed managed response (a full envelope JSON line) back
        /// to the broker, which routes it to the requesting client.
        ///
        /// # Safety
        /// `id`/`response` must each be valid for their stated lengths or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_complete_request(
            id: *const u8,
            id_len: i32,
            response: *const u8,
            response_len: i32,
        ) {
            let id = $crate::ffi::string_from_raw(id, id_len);
            let response = $crate::ffi::slice_from_raw(response, response_len).to_vec();
            $crate::api::complete_request(&id, response);
        }

        /// Emit an unsolicited event (no `reply_to`) to every connected client.
        ///
        /// # Safety
        /// `event_type`/`payload` must each be valid for their stated lengths or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_emit_event(
            event_type: *const u8,
            type_len: i32,
            payload: *const u8,
            payload_len: i32,
        ) {
            let event_type = $crate::ffi::string_from_raw(event_type, type_len);
            if event_type.is_empty() {
                return;
            }
            $crate::api::emit_event(&event_type, $crate::ffi::string_from_raw(payload, payload_len));
        }

        /// 1 when at least one Locus client is connected.
        #[no_mangle]
        pub extern "C" fn locus_has_connected_client() -> ::std::os::raw::c_int {
            ::std::os::raw::c_int::from($crate::api::connected())
        }

        /// Monotonic count of cancel signals.
        #[no_mangle]
        pub extern "C" fn locus_execution_cancel_seq() -> u64 {
            $crate::api::execution_cancel_seq()
        }

        /// 1 when a cancel newer than `since_seq` targets `execution_id` (or
        /// every execution). Safe from any thread.
        ///
        /// # Safety
        /// `execution_id` must be valid for `execution_id_len` bytes or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_execution_cancel_requested(
            execution_id: *const u8,
            execution_id_len: i32,
            since_seq: u64,
        ) -> ::std::os::raw::c_int {
            let execution_id = $crate::ffi::string_from_raw(execution_id, execution_id_len);
            ::std::os::raw::c_int::from($crate::api::execution_cancel_requested(&execution_id, since_seq))
        }

        /// Milliseconds since the host loop last pulsed, or -1 when unknown.
        #[no_mangle]
        pub extern "C" fn locus_main_thread_stall_ms() -> i64 {
            $crate::api::main_thread_stall_ms()
        }

        /// Publish the latest progress snapshot (JSON object) of a running
        /// execution, from a monitor thread.
        ///
        /// # Safety
        /// `execution_id`/`json` must each be valid for their stated lengths or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_execution_publish(
            execution_id: *const u8,
            execution_id_len: i32,
            json: *const u8,
            json_len: i32,
        ) {
            $crate::api::execution_publish(
                &$crate::ffi::string_from_raw(execution_id, execution_id_len),
                &$crate::ffi::string_from_raw(json, json_len),
            );
        }

        /// Forget an execution's progress snapshot.
        ///
        /// # Safety
        /// `execution_id` must be valid for `execution_id_len` bytes or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_execution_end(execution_id: *const u8, execution_id_len: i32) {
            $crate::api::execution_end(&$crate::ffi::string_from_raw(execution_id, execution_id_len));
        }

        /// Milliseconds since a client last polled this execution's progress
        /// through the broker, or -1 when none did.
        ///
        /// # Safety
        /// `execution_id` must be valid for `execution_id_len` bytes or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_execution_client_poll_age_ms(
            execution_id: *const u8,
            execution_id_len: i32,
        ) -> i64 {
            $crate::api::execution_client_poll_age_ms(&$crate::ffi::string_from_raw(
                execution_id,
                execution_id_len,
            ))
        }
    };
}

/// Entry points added for hosts that load the DLL through a foreign function
/// interface without a fixed project path (Blender). Unity's DLL does not
/// export these, so its symbol list stays exactly the original one.
#[macro_export]
macro_rules! export_host_ffi {
    ($config:path) => {
        /// Initialize with explicit names: `$config(instance_key, pipe,
        /// state_plane, protocol)` builds the configuration.
        ///
        /// # Safety
        /// Each pointer must be valid for its stated length or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_init_host(
            instance_key: *const u8,
            instance_key_len: i32,
            pipe: *const u8,
            pipe_len: i32,
            state_plane: *const u8,
            state_plane_len: i32,
            protocol_version: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let instance_key = $crate::ffi::string_from_raw(instance_key, instance_key_len);
            let pipe = $crate::ffi::string_from_raw(pipe, pipe_len);
            let state_plane = $crate::ffi::string_from_raw(state_plane, state_plane_len);
            if pipe.trim().is_empty() {
                return -1;
            }
            match $config(instance_key, pipe, state_plane, protocol_version) {
                Some(config) => $crate::api::init(config),
                None => -1,
            }
        }

        /// Block until a request is queued (1), or timeout/shutdown (0); -1
        /// without a broker. ctypes releases the GIL for the call.
        #[no_mangle]
        pub extern "C" fn locus_wait_request(timeout_ms: i32) -> ::std::os::raw::c_int {
            $crate::api::wait_request(timeout_ms)
        }

        /// Merge a host JSON object into the status snapshot as `host`.
        ///
        /// # Safety
        /// `json` must be valid for `json_len` bytes or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_set_host_info(json: *const u8, json_len: i32) {
            $crate::api::set_host_info(&$crate::ffi::string_from_raw(json, json_len));
        }

        /// Require `hello <token>` as the first frame of every connection.
        /// Call right after `locus_init_host`, before publishing the pipe.
        ///
        /// # Safety
        /// `token` must be valid for `token_len` bytes or null.
        #[no_mangle]
        pub unsafe extern "C" fn locus_set_auth_token(token: *const u8, token_len: i32) {
            $crate::api::set_auth_token(&$crate::ffi::string_from_raw(token, token_len));
        }

        /// Broker protocol version (lets the host verify the DLL it loaded).
        #[no_mangle]
        pub extern "C" fn locus_protocol_version() -> ::std::os::raw::c_int {
            $crate::protocol::NATIVE_PROTOCOL_VERSION
        }
    };
}
