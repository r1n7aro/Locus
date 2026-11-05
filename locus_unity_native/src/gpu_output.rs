//! Small C ABI over the shared GPU SDK, bundled in the existing native plugin.
use locus_gpu_transport::{producer, protocol::InputEvent};
use std::{cell::RefCell, ffi::c_void};
thread_local! {static ERROR:RefCell<String>=const {RefCell::new(String::new())};}
fn record(error: impl ToString) {
    ERROR.with(|e| *e.borrow_mut() = error.to_string());
}
fn checked<T>(operation: impl FnOnce() -> Result<T, String>, fallback: T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => value,
        Ok(Err(error)) => {
            record(error);
            fallback
        }
        Err(_) => {
            record("GPU transport panicked");
            fallback
        }
    }
}
#[no_mangle]
pub extern "C" fn locus_render_version() -> i32 {
    1
}
#[no_mangle]
pub unsafe extern "C" fn locus_render_create(id: *const u8, length: i32) -> u64 {
    checked(
        || {
            if id.is_null() || length != 36 {
                return Err("Invalid GPU output id".into());
            }
            let id = std::str::from_utf8(std::slice::from_raw_parts(id, length as usize))
                .map_err(|e| e.to_string())?;
            producer::create(id)
        },
        0,
    )
}
#[no_mangle]
pub unsafe extern "C" fn locus_render_bind(handle: u64, texture: *mut c_void) -> i32 {
    checked(
        || {
            producer::bind(handle, texture)?;
            Ok(0)
        },
        -1,
    )
}
#[no_mangle]
pub extern "C" fn locus_render_enqueue(handle: u64) -> i64 {
    checked(|| producer::enqueue(handle).map(|token| token as i64), -1)
}
#[no_mangle]
pub extern "C" fn locus_render_cancel(token: usize) {
    producer::cancel_packet(token);
}
#[no_mangle]
pub extern "C" fn locus_render_callback() -> producer::RenderCallback {
    producer::callback()
}
#[no_mangle]
pub extern "C" fn locus_render_close(handle: u64) {
    producer::close(handle);
}
#[no_mangle]
pub unsafe extern "C" fn locus_render_input(handle: u64, event: *mut InputEvent) -> i32 {
    checked(
        || {
            if event.is_null() {
                return Err("Missing input buffer".into());
            }
            match producer::poll_input(handle)? {
                Some(value) => {
                    *event = value;
                    Ok(1)
                }
                None => Ok(0),
            }
        },
        -1,
    )
}
#[no_mangle]
pub unsafe extern "C" fn locus_render_state(handle: u64, state: *mut producer::OutputState) -> i32 {
    checked(
        || {
            if state.is_null() {
                return Err("Missing state buffer".into());
            }
            *state = producer::state(handle)?;
            Ok(0)
        },
        -1,
    )
}
#[no_mangle]
pub extern "C" fn locus_render_wait(handle: u64, milliseconds: u32) -> i32 {
    i32::from(producer::wait(handle, milliseconds))
}
#[no_mangle]
pub extern "C" fn locus_render_ack_input(handle: u64, sequence: u64, timestamp: u64) -> i32 {
    checked(
        || producer::acknowledge_input(handle, sequence, timestamp).map(|()| 0),
        -1,
    )
}
#[no_mangle]
pub unsafe extern "C" fn locus_render_error(buffer: *mut u8, capacity: i32) -> i32 {
    if buffer.is_null() || capacity <= 0 {
        return 0;
    }
    ERROR.with(|error| {
        let error = error.borrow();
        let n = error.len().min(capacity as usize);
        std::ptr::copy_nonoverlapping(error.as_ptr(), buffer, n);
        n as i32
    })
}
