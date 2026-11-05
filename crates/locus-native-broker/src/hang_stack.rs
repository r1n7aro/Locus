//! Main-thread stack capture for hang diagnosis.
//!
//! When the host main thread stops pumping its loop, Locus asks the broker —
//! which runs on its own thread and keeps answering — where the main thread
//! is. Capturing from inside the process is what makes the answer useful:
//! JIT runtimes such as Mono register unwind data with the OS, so an
//! in-process unwinder walks straight through managed frames, and the host's
//! [`ManagedNamer`] (Unity: Mono's jit-info table) names them.
//!
//! Safety model (the editor must never be made worse by a diagnosis):
//!   * The suspended window only runs lock-free work: `GetThreadContext`,
//!     `VirtualQuery` (a syscall) and one memcpy into a buffer allocated before
//!     the suspend. No allocation, no loader/heap/Mono lock, so the main thread
//!     can never be left frozen behind a lock this thread is waiting on.
//!   * Unwinding runs after the thread is resumed, against the private stack
//!     copy, and every stack read is bounds-checked against that copy. A
//!     corrupt chain ends the walk instead of faulting.
//!   * Managed naming runs on a separate helper thread with a deadline. If the
//!     runtime is wedged behind a lock the main thread holds, the capture still
//!     returns the native frames and reports the naming step as timed out.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use crate::hang_stack_types::{
    CaptureRequest, CaptureResult, FrameOut, ManagedFrame, ManagedNamer, SampleOut,
};

type Bool = i32;
type Dword = u32;
type Handle = *mut c_void;

const THREAD_SUSPEND_RESUME: Dword = 0x0002;
const THREAD_GET_CONTEXT: Dword = 0x0008;
const THREAD_QUERY_INFORMATION: Dword = 0x0040;
const CONTEXT_AMD64: Dword = 0x0010_0000;
const CONTEXT_CONTROL: Dword = CONTEXT_AMD64 | 0x1;
const CONTEXT_INTEGER: Dword = CONTEXT_AMD64 | 0x2;
const MEM_COMMIT: Dword = 0x1000;
const PAGE_NOACCESS: Dword = 0x01;
const PAGE_GUARD: Dword = 0x100;
const PAGE_EXECUTE_ANY: Dword = 0x10 | 0x20 | 0x40 | 0x80;

/// Bytes of the main-thread stack copied per sample (from RSP upwards).
const MAX_STACK_COPY: usize = 1024 * 1024;
const DEFAULT_SAMPLES: u32 = 3;
const MAX_SAMPLES: u32 = 8;
const DEFAULT_INTERVAL_MS: u64 = 250;
const DEFAULT_MAX_FRAMES: usize = 96;
const MAX_FRAMES_LIMIT: usize = 256;
const MANAGED_NAMING_TIMEOUT: Duration = Duration::from_secs(3);
/// Top-frame fallback: slots scanned for a return address when the
/// instruction pointer sits in code without unwind data (trampolines).
const TOP_FRAME_SCAN_SLOTS: u64 = 64;

const RSP: usize = 4;

const UWOP_PUSH_NONVOL: u8 = 0;
const UWOP_ALLOC_LARGE: u8 = 1;
const UWOP_ALLOC_SMALL: u8 = 2;
const UWOP_SET_FPREG: u8 = 3;
const UWOP_SAVE_NONVOL: u8 = 4;
const UWOP_SAVE_NONVOL_FAR: u8 = 5;
const UWOP_EPILOG: u8 = 6;
const UWOP_SAVE_XMM128: u8 = 8;
const UWOP_SAVE_XMM128_FAR: u8 = 9;
const UWOP_PUSH_MACHFRAME: u8 = 10;
const UNW_FLAG_CHAININFO: u8 = 0x4;

// x64 CONTEXT (layout pinned by the asserts below).
#[allow(dead_code)]
#[repr(C, align(16))]
struct Context {
    p1_home: u64,
    p2_home: u64,
    p3_home: u64,
    p4_home: u64,
    p5_home: u64,
    p6_home: u64,
    context_flags: u32,
    mx_csr: u32,
    seg_cs: u16,
    seg_ds: u16,
    seg_es: u16,
    seg_fs: u16,
    seg_gs: u16,
    seg_ss: u16,
    eflags: u32,
    dr0: u64,
    dr1: u64,
    dr2: u64,
    dr3: u64,
    dr6: u64,
    dr7: u64,
    rax: u64,
    rcx: u64,
    rdx: u64,
    rbx: u64,
    rsp: u64,
    rbp: u64,
    rsi: u64,
    rdi: u64,
    r8: u64,
    r9: u64,
    r10: u64,
    r11: u64,
    r12: u64,
    r13: u64,
    r14: u64,
    r15: u64,
    rip: u64,
    flt_save: [u8; 512],
    vector_register: [u8; 16 * 26],
    vector_control: u64,
    debug_control: u64,
    last_branch_to_rip: u64,
    last_branch_from_rip: u64,
    last_exception_to_rip: u64,
    last_exception_from_rip: u64,
}

const _: () = {
    assert!(core::mem::offset_of!(Context, context_flags) == 0x30);
    assert!(core::mem::offset_of!(Context, rax) == 0x78);
    assert!(core::mem::offset_of!(Context, rsp) == 0x98);
    assert!(core::mem::offset_of!(Context, rip) == 0xF8);
    assert!(size_of::<Context>() == 1232);
};

#[repr(C)]
struct RuntimeFunction {
    begin: u32,
    end: u32,
    unwind_data: u32,
}

#[repr(C)]
struct MemoryBasicInformation {
    base_address: *mut c_void,
    allocation_base: *mut c_void,
    allocation_protect: u32,
    partition_id: u16,
    region_size: usize,
    state: u32,
    protect: u32,
    kind: u32,
}

#[repr(C)]
struct Filetime {
    low: Dword,
    high: Dword,
}

unsafe extern "system" {
    fn OpenThread(access: Dword, inherit: Bool, tid: Dword) -> Handle;
    fn CloseHandle(h: Handle) -> Bool;
    fn SuspendThread(h: Handle) -> Dword;
    fn ResumeThread(h: Handle) -> Dword;
    fn GetThreadContext(h: Handle, ctx: *mut Context) -> Bool;
    fn GetThreadTimes(
        h: Handle,
        creation: *mut Filetime,
        exit: *mut Filetime,
        kernel: *mut Filetime,
        user: *mut Filetime,
    ) -> Bool;
    fn GetCurrentThreadId() -> Dword;
    fn VirtualQuery(
        address: *const c_void,
        buffer: *mut MemoryBasicInformation,
        length: usize,
    ) -> usize;
    fn RtlLookupFunctionEntry(
        control_pc: u64,
        image_base: *mut u64,
        history_table: *mut c_void,
    ) -> *mut RuntimeFunction;
}

struct OwnedHandle(Handle);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

// ── Capture ─────────────────────────────────────────────────────────────────

static CAPTURE_BUSY: AtomicBool = AtomicBool::new(false);
static MANAGED_NAMING_BUSY: AtomicBool = AtomicBool::new(false);

struct BusyGuard(&'static AtomicBool);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Resolve the host's managed-runtime entry points while still on the thread
/// that initialized the broker (the host main thread), so later captures
/// never need a loader lookup while the main thread might own the loader lock.
pub fn prepare(namer: Option<&'static dyn ManagedNamer>) {
    if let Some(namer) = namer {
        namer.prepare();
    }
}

/// `generation` is the managed generation; managed names cached for it are
/// reused so repeated samples do not attach to the runtime again.
pub fn capture(
    thread_id: u32,
    request: &CaptureRequest,
    generation: i64,
    namer: Option<&'static dyn ManagedNamer>,
) -> Result<CaptureResult, String> {
    if thread_id == 0 {
        return Err("main thread id is unknown".to_string());
    }
    if thread_id == unsafe { GetCurrentThreadId() } {
        return Err("refusing to capture the calling thread".to_string());
    }
    if CAPTURE_BUSY.swap(true, Ordering::SeqCst) {
        return Err("capture_in_progress".to_string());
    }
    let _busy = BusyGuard(&CAPTURE_BUSY);

    let samples = request.samples.unwrap_or(DEFAULT_SAMPLES).clamp(1, MAX_SAMPLES);
    let interval = Duration::from_millis(
        request
            .interval_ms
            .unwrap_or(DEFAULT_INTERVAL_MS)
            .clamp(10, 2_000),
    );
    let max_frames = request
        .max_frames
        .unwrap_or(DEFAULT_MAX_FRAMES)
        .clamp(4, MAX_FRAMES_LIMIT);

    let raw = unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION,
            0,
            thread_id,
        )
    };
    if raw.is_null() {
        return Err(format!(
            "OpenThread({thread_id}) failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let thread = OwnedHandle(raw);

    // Everything the suspended window touches is allocated up front.
    let mut stack_copy = vec![0u8; MAX_STACK_COPY];
    let mut context: Box<Context> = Box::new(unsafe { zeroed() });

    let started = Instant::now();
    let started_at_ms = now_ms();
    let cpu_start_100ns = thread_cpu_100ns(thread.0);
    let mut out = Vec::with_capacity(samples as usize);
    for index in 0..samples {
        if index > 0 {
            std::thread::sleep(interval);
        }
        let captured_at_ms = now_ms();
        match unsafe { snapshot(thread.0, &mut context, &mut stack_copy) } {
            Ok(snap) => {
                let stack = &stack_copy[..snap.stack_len];
                let (frames, unwind_stop) = unwind(&snap, stack, max_frames);
                out.push(SampleOut {
                    captured_at_ms,
                    suspend_us: snap.suspend_us,
                    stack_bytes: snap.stack_len,
                    frames,
                    unwind_stop: unwind_stop.map(str::to_string),
                    error: None,
                });
            }
            Err(error) => out.push(SampleOut {
                captured_at_ms,
                suspend_us: 0,
                stack_bytes: 0,
                frames: Vec::new(),
                unwind_stop: None,
                error: Some(error),
            }),
        }
    }
    let cpu_end_100ns = thread_cpu_100ns(thread.0);
    let wall_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;

    let (managed, managed_status, managed_error) = if request.resolve_managed == Some(false) {
        (Vec::new(), "skipped".to_string(), None)
    } else {
        resolve_managed_frames(&out, generation, namer)
    };

    Ok(CaptureResult {
        version: 1,
        thread_id,
        process_id: std::process::id(),
        started_at_ms,
        wall_ms,
        cpu_start_100ns,
        cpu_end_100ns,
        samples: out,
        managed,
        managed_status,
        managed_error,
    })
}

fn thread_cpu_100ns(thread: Handle) -> Option<u64> {
    let (mut c, mut e, mut k, mut u): (Filetime, Filetime, Filetime, Filetime) =
        unsafe { (zeroed(), zeroed(), zeroed(), zeroed()) };
    let ok = unsafe { GetThreadTimes(thread, &mut c, &mut e, &mut k, &mut u) != 0 };
    ok.then(|| {
        let kernel = ((k.high as u64) << 32) | k.low as u64;
        let user = ((u.high as u64) << 32) | u.low as u64;
        kernel + user
    })
}

struct Snapshot {
    regs: [u64; 16],
    rip: u64,
    /// Address the copy starts at (the captured RSP).
    stack_base: u64,
    stack_len: usize,
    suspend_us: u64,
}

/// Freeze the thread just long enough to copy its registers and live stack.
///
/// # Safety
/// `thread` must be a handle with suspend/get-context rights that does not
/// refer to the calling thread.
unsafe fn snapshot(
    thread: Handle,
    context: &mut Context,
    buffer: &mut [u8],
) -> Result<Snapshot, String> {
    *context = zeroed();
    context.context_flags = CONTEXT_CONTROL | CONTEXT_INTEGER;
    let started = Instant::now();
    if SuspendThread(thread) == u32::MAX {
        return Err(format!(
            "SuspendThread failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // ── suspended: no allocation, no locks ──
    let context_ok = GetThreadContext(thread, context) != 0;
    let context_error = (!context_ok).then(std::io::Error::last_os_error);
    let mut stack_len = 0usize;
    if context_ok {
        let rsp = context.rsp;
        let mut info: MemoryBasicInformation = zeroed();
        let queried = VirtualQuery(
            rsp as *const c_void,
            &mut info,
            size_of::<MemoryBasicInformation>(),
        ) != 0;
        if queried
            && info.state == MEM_COMMIT
            && info.protect & (PAGE_GUARD | PAGE_NOACCESS) == 0
        {
            let region_end = (info.base_address as u64).saturating_add(info.region_size as u64);
            if region_end > rsp {
                stack_len = ((region_end - rsp) as usize).min(buffer.len());
                std::ptr::copy_nonoverlapping(rsp as *const u8, buffer.as_mut_ptr(), stack_len);
            }
        }
    }
    ResumeThread(thread);
    // ── resumed ──
    let suspend_us = started.elapsed().as_micros().min(u64::MAX as u128) as u64;
    if let Some(error) = context_error {
        return Err(format!("GetThreadContext failed: {error}"));
    }
    Ok(Snapshot {
        regs: [
            context.rax,
            context.rcx,
            context.rdx,
            context.rbx,
            context.rsp,
            context.rbp,
            context.rsi,
            context.rdi,
            context.r8,
            context.r9,
            context.r10,
            context.r11,
            context.r12,
            context.r13,
            context.r14,
            context.r15,
        ],
        rip: context.rip,
        stack_base: context.rsp,
        stack_len,
        suspend_us,
    })
}

// ── Unwinder (runs on the private stack copy) ───────────────────────────────

struct StackView<'a> {
    base: u64,
    bytes: &'a [u8],
}

impl StackView<'_> {
    fn read(&self, address: u64) -> Option<u64> {
        let offset = address.checked_sub(self.base)? as usize;
        let end = offset.checked_add(8)?;
        let slice = self.bytes.get(offset..end)?;
        Some(u64::from_le_bytes(slice.try_into().ok()?))
    }

    fn end(&self) -> u64 {
        self.base + self.bytes.len() as u64
    }
}

fn region_readable(address: u64, executable: bool) -> bool {
    if address < 0x10000 {
        return false;
    }
    let mut info: MemoryBasicInformation = unsafe { zeroed() };
    let queried = unsafe {
        VirtualQuery(
            address as *const c_void,
            &mut info,
            size_of::<MemoryBasicInformation>(),
        ) != 0
    };
    queried
        && info.state == MEM_COMMIT
        && info.protect & (PAGE_GUARD | PAGE_NOACCESS) == 0
        && (!executable || info.protect & PAGE_EXECUTE_ANY != 0)
}

fn is_code_address(address: u64) -> bool {
    region_readable(address, true)
}

/// Copy `N` code bytes starting at `address`, after checking both ends are
/// committed and readable.
fn read_code<const N: usize>(address: u64) -> Option<[u8; N]> {
    if !region_readable(address, false) || !region_readable(address.checked_add(N as u64 - 1)?, false) {
        return None;
    }
    let mut out = [0u8; N];
    unsafe {
        std::ptr::copy_nonoverlapping(address as *const u8, out.as_mut_ptr(), N);
    }
    Some(out)
}

fn unwind(snap: &Snapshot, stack: &[u8], max_frames: usize) -> (Vec<FrameOut>, Option<&'static str>) {
    let view = StackView {
        base: snap.stack_base,
        bytes: stack,
    };
    let mut regs = snap.regs;
    let mut rip = snap.rip;
    let mut frames = Vec::with_capacity(max_frames.min(128));
    let mut stop = None;
    loop {
        if rip == 0 {
            break;
        }
        if frames.len() >= max_frames {
            stop = Some("max_frames");
            break;
        }
        frames.push(FrameOut {
            ip: rip,
            sp: regs[RSP],
        });
        let previous_rsp = regs[RSP];
        let top = frames.len() == 1;
        if let Err(reason) = unsafe { step(&mut regs, &mut rip, &view, top) } {
            stop = Some(reason);
            break;
        }
        if regs[RSP] <= previous_rsp {
            stop = Some("stack_not_advancing");
            break;
        }
        if rip != 0 && !is_code_address(rip) {
            stop = Some("invalid_return_address");
            break;
        }
        if regs[RSP] >= view.end() && rip != 0 {
            // The caller frame is known but its own frame lies beyond the copy.
            if frames.len() < max_frames {
                frames.push(FrameOut {
                    ip: rip,
                    sp: regs[RSP],
                });
            }
            stop = Some("stack_copy_end");
            break;
        }
    }
    (frames, stop)
}

fn opcode_size(op: u8, info: usize) -> usize {
    match op {
        UWOP_ALLOC_LARGE => 2 + usize::from(info != 0),
        UWOP_SAVE_NONVOL | UWOP_SAVE_XMM128 | UWOP_EPILOG => 2,
        UWOP_SAVE_NONVOL_FAR | UWOP_SAVE_XMM128_FAR => 3,
        _ => 1,
    }
}

/// Unwind one frame. Mirrors the x64 `RtlVirtualUnwind` algorithm (prolog
/// codes, chained info, epilog emulation) but reads the stack only through the
/// bounds-checked copy.
///
/// # Safety
/// Reads unwind data and code bytes of the current process.
unsafe fn step(
    regs: &mut [u64; 16],
    rip: &mut u64,
    stack: &StackView<'_>,
    top_frame: bool,
) -> Result<(), &'static str> {
    let pc = *rip;
    let mut image_base = 0u64;
    let entry = RtlLookupFunctionEntry(pc, &mut image_base, null_mut());
    if entry.is_null() {
        // Leaf function or code without unwind data: the return address is
        // on top of the stack.
        let ret = stack.read(regs[RSP]).ok_or("stack_end")?;
        if top_frame && !is_code_address(ret) {
            // A trampoline (no unwind data) that already adjusted RSP: find the
            // nearest plausible return address instead of giving up.
            for slot in 1..TOP_FRAME_SCAN_SLOTS {
                let address = regs[RSP] + slot * 8;
                let Some(candidate) = stack.read(address) else { break };
                if is_code_address(candidate) {
                    *rip = candidate;
                    regs[RSP] = address + 8;
                    return Ok(());
                }
            }
        }
        *rip = ret;
        regs[RSP] += 8;
        return Ok(());
    }

    let mut function = entry as *const RuntimeFunction;
    let mut machine_frame = false;
    for _ in 0..32 {
        let f = &*function;
        let info = (image_base + u64::from(f.unwind_data)) as *const u8;
        let version_flags = *info;
        let version = version_flags & 0x7;
        let flags = version_flags >> 3;
        if version != 1 && version != 2 {
            return Err("unwind_version");
        }
        let prolog = u64::from(*info.add(1));
        let count = usize::from(*info.add(2));
        let frame_byte = *info.add(3);
        let frame_reg = usize::from(frame_byte & 0x0F);
        let frame_offset = u64::from(frame_byte >> 4) * 16;
        let codes = info.add(4) as *const u16;
        let code = |index: usize| -> u16 { codes.add(index).read_unaligned() };

        let frame = if frame_reg == 0 {
            regs[RSP]
        } else {
            regs[frame_reg].wrapping_sub(frame_offset)
        };
        let begin = image_base + u64::from(f.begin);
        let end = image_base + u64::from(f.end);
        let prolog_offset = if pc >= begin && pc < begin + prolog {
            pc - begin
        } else {
            if count > 0 && is_inside_epilog(pc, begin, end) {
                return interpret_epilog(pc, regs, rip, stack);
            }
            u64::MAX
        };

        let mut index = 0usize;
        while index < count {
            let raw = code(index);
            let offset = u64::from(raw & 0xFF);
            let op = ((raw >> 8) & 0x0F) as u8;
            let op_info = usize::from(raw >> 12);
            let size = opcode_size(op, op_info);
            if index + size > count {
                return Err("unwind_codes_truncated");
            }
            if prolog_offset < offset {
                index += size;
                continue;
            }
            match op {
                UWOP_PUSH_NONVOL => {
                    regs[op_info] = stack.read(regs[RSP]).ok_or("stack_end")?;
                    regs[RSP] += 8;
                }
                UWOP_ALLOC_LARGE => {
                    let amount = if op_info == 0 {
                        u64::from(code(index + 1)) * 8
                    } else {
                        u64::from(code(index + 1)) | (u64::from(code(index + 2)) << 16)
                    };
                    regs[RSP] = regs[RSP].wrapping_add(amount);
                }
                UWOP_ALLOC_SMALL => {
                    regs[RSP] += (op_info as u64 + 1) * 8;
                }
                UWOP_SET_FPREG => {
                    regs[RSP] = frame;
                }
                UWOP_SAVE_NONVOL => {
                    let address = frame.wrapping_add(u64::from(code(index + 1)) * 8);
                    regs[op_info] = stack.read(address).ok_or("stack_end")?;
                }
                UWOP_SAVE_NONVOL_FAR => {
                    let offset = u64::from(code(index + 1)) | (u64::from(code(index + 2)) << 16);
                    regs[op_info] = stack.read(frame.wrapping_add(offset)).ok_or("stack_end")?;
                }
                UWOP_PUSH_MACHFRAME => {
                    if op_info != 0 {
                        regs[RSP] += 8;
                    }
                    *rip = stack.read(regs[RSP]).ok_or("stack_end")?;
                    regs[RSP] = stack.read(regs[RSP] + 24).ok_or("stack_end")?;
                    machine_frame = true;
                }
                _ => {}
            }
            index += size;
        }

        if flags & UNW_FLAG_CHAININFO == 0 {
            break;
        }
        function = codes.add((count + 1) & !1) as *const RuntimeFunction;
    }

    if !machine_frame {
        *rip = stack.read(regs[RSP]).ok_or("stack_end")?;
        regs[RSP] += 8;
    }
    Ok(())
}

/// Same recognizer the OS unwinder uses: optional `add/lea rsp`, then pops,
/// then `ret` (or a jump that stays inside the function).
fn is_inside_epilog(pc: u64, begin: u64, end: u64) -> bool {
    let mut pc = pc;
    let Some(head) = read_code::<8>(pc) else { return false };
    if head[0] & 0xF8 == 0x48 {
        match head[1] {
            0x81 if head[0] == 0x48 && head[2] == 0xC4 => pc += 7,
            0x83 if head[0] == 0x48 && head[2] == 0xC4 => pc += 4,
            0x8D => {
                if head[0] & 0x06 != 0 || (head[2] >> 3) & 7 != 4 || head[2] & 7 == 4 {
                    return false;
                }
                match head[2] >> 6 {
                    1 => pc += 4,
                    2 => pc += 7,
                    _ => return false,
                }
            }
            0x81 | 0x83 => return false,
            _ => {}
        }
    }
    for _ in 0..32 {
        let Some(bytes) = read_code::<6>(pc) else { return false };
        let (op, at) = if bytes[0] & 0xF0 == 0x40 { (bytes[1], 1usize) } else { (bytes[0], 0usize) };
        match op {
            0x58..=0x5F => {
                pc += at as u64 + 1;
            }
            0xC2 | 0xC3 => return true,
            0xE9 if at == 0 => {
                let rel = i32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
                pc = pc.wrapping_add(5).wrapping_add(rel as i64 as u64);
                if pc < begin || pc >= end {
                    return false;
                }
            }
            0xEB if at == 0 => {
                pc = pc.wrapping_add(2).wrapping_add(bytes[1] as i8 as i64 as u64);
                if pc < begin || pc >= end {
                    return false;
                }
            }
            0xF3 if at == 0 => return bytes[1] == 0xC3,
            _ => return false,
        }
    }
    false
}

fn interpret_epilog(
    pc: u64,
    regs: &mut [u64; 16],
    rip: &mut u64,
    stack: &StackView<'_>,
) -> Result<(), &'static str> {
    let mut pc = pc;
    for _ in 0..32 {
        let bytes = read_code::<8>(pc).ok_or("epilog_unreadable")?;
        let (rex, at) = if bytes[0] & 0xF0 == 0x40 { (bytes[0] & 0x0F, 1usize) } else { (0, 0usize) };
        let b = &bytes[at..];
        match b[0] {
            0x58..=0x5F => {
                let reg = usize::from(b[0] - 0x58) + usize::from(rex & 1) * 8;
                regs[reg] = stack.read(regs[RSP]).ok_or("stack_end")?;
                regs[RSP] += 8;
                pc += at as u64 + 1;
            }
            0x81 => {
                let value = i32::from_le_bytes([b[2], b[3], b[4], b[5]]);
                regs[RSP] = regs[RSP].wrapping_add(value as i64 as u64);
                pc += at as u64 + 6;
            }
            0x83 => {
                regs[RSP] = regs[RSP].wrapping_add(b[2] as i8 as i64 as u64);
                pc += at as u64 + 3;
            }
            0x8D => {
                let reg = usize::from(b[1] & 7) + usize::from(rex & 1) * 8;
                if b[1] >> 6 == 1 {
                    regs[RSP] = regs[reg].wrapping_add(b[2] as i8 as i64 as u64);
                    pc += at as u64 + 3;
                } else {
                    let value = i32::from_le_bytes([b[2], b[3], b[4], b[5]]);
                    regs[RSP] = regs[reg].wrapping_add(value as i64 as u64);
                    pc += at as u64 + 6;
                }
            }
            0xC2 => {
                *rip = stack.read(regs[RSP]).ok_or("stack_end")?;
                regs[RSP] += 8 + u64::from(u16::from_le_bytes([b[1], b[2]]));
                return Ok(());
            }
            0xC3 | 0xF3 => {
                *rip = stack.read(regs[RSP]).ok_or("stack_end")?;
                regs[RSP] += 8;
                return Ok(());
            }
            0xE9 => {
                let rel = i32::from_le_bytes([b[1], b[2], b[3], b[4]]);
                pc = pc.wrapping_add(at as u64 + 5).wrapping_add(rel as i64 as u64);
            }
            0xEB => {
                pc = pc.wrapping_add(at as u64 + 2).wrapping_add(b[1] as i8 as i64 as u64);
            }
            _ => return Err("epilog_unsupported"),
        }
    }
    Err("epilog_too_long")
}

// ── Managed naming ──────────────────────────────────────────────────────────

/// `(frame ip, address to look up)`. Return addresses are looked up one byte
/// back so a call that ends its method still resolves to that method.
fn managed_lookups(samples: &[SampleOut]) -> Vec<(u64, u64)> {
    let mut seen = std::collections::HashSet::new();
    let mut lookups = Vec::new();
    for sample in samples {
        for (index, frame) in sample.frames.iter().enumerate() {
            let lookup = if index == 0 { frame.ip } else { frame.ip.saturating_sub(1) };
            if seen.insert((frame.ip, lookup)) {
                lookups.push((frame.ip, lookup));
            }
        }
    }
    lookups
}

/// Managed names resolved earlier in this managed generation, including
/// addresses the runtime did not own (`None`). JIT code is freed when the
/// generation ends, so a new one starts empty. Repeated samples of one
/// incident then name nothing new and never attach a thread again.
struct NameCache {
    generation: i64,
    names: HashMap<(u64, u64), Option<ManagedFrame>>,
}

const NAME_CACHE_LIMIT: usize = 8192;

fn name_cache() -> &'static Mutex<NameCache> {
    static CACHE: OnceLock<Mutex<NameCache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(NameCache {
            generation: i64::MIN,
            names: HashMap::new(),
        })
    })
}

/// Split lookups into names already known for `generation` and lookups that
/// still need the runtime.
fn cached_names(
    generation: i64,
    lookups: Vec<(u64, u64)>,
) -> (Vec<ManagedFrame>, Vec<(u64, u64)>) {
    let Ok(mut cache) = name_cache().lock() else {
        return (Vec::new(), lookups);
    };
    if cache.generation != generation {
        cache.generation = generation;
        cache.names.clear();
    }
    let mut known = Vec::new();
    let mut missing = Vec::new();
    for key in lookups {
        match cache.names.get(&key) {
            Some(Some(frame)) => known.push(frame.clone()),
            Some(None) => {}
            None => missing.push(key),
        }
    }
    (known, missing)
}

fn remember_names(generation: i64, looked_up: &[(u64, u64)], named: &[ManagedFrame]) {
    let Ok(mut cache) = name_cache().lock() else { return };
    if cache.generation != generation {
        return;
    }
    if cache.names.len().saturating_add(looked_up.len()) > NAME_CACHE_LIMIT {
        cache.names.clear();
    }
    for &(ip, lookup) in looked_up {
        let frame = named.iter().find(|frame| frame.ip == ip).cloned();
        cache.names.insert((ip, lookup), frame);
    }
}

fn resolve_managed_frames(
    samples: &[SampleOut],
    generation: i64,
    namer: Option<&'static dyn ManagedNamer>,
) -> (Vec<ManagedFrame>, String, Option<String>) {
    let lookups = managed_lookups(samples);
    if lookups.is_empty() {
        return (Vec::new(), "skipped".to_string(), None);
    }
    let (mut known, lookups) = cached_names(generation, lookups);
    if lookups.is_empty() {
        return (known, "ok".to_string(), None);
    }
    let Some(namer) = namer else {
        return (
            known,
            "unavailable".to_string(),
            Some("this host has no managed runtime to name frames".to_string()),
        );
    };
    if let Some(reason) = namer.unavailable_reason() {
        return (known, "unavailable".to_string(), Some(reason.to_string()));
    }
    if MANAGED_NAMING_BUSY.swap(true, Ordering::SeqCst) {
        // A previous naming thread is still blocked inside the runtime.
        return (
            known,
            "blocked".to_string(),
            Some("a previous managed lookup has not returned; the runtime is likely blocked by the main thread".to_string()),
        );
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let thread_lookups = lookups.clone();
    let spawned = std::thread::Builder::new()
        .name("locus-hang-names".to_string())
        .spawn(move || {
            let _busy = BusyGuard(&MANAGED_NAMING_BUSY);
            let result = unsafe { namer.name(&thread_lookups) };
            let _ = tx.send(result);
        });
    if let Err(error) = spawned {
        MANAGED_NAMING_BUSY.store(false, Ordering::SeqCst);
        return (known, "error".to_string(), Some(format!("spawn failed: {error}")));
    }
    match rx.recv_timeout(MANAGED_NAMING_TIMEOUT) {
        Ok(Ok(frames)) => {
            // Only a completed lookup may record misses; a timeout or error
            // must not hide frames the runtime could name later.
            remember_names(generation, &lookups, &frames);
            known.extend(frames);
            (known, "ok".to_string(), None)
        }
        Ok(Err(error)) => (known, "error".to_string(), Some(error)),
        Err(_) => (
            known,
            "timeout".to_string(),
            Some(format!(
                "the managed runtime did not answer within {}ms; it is likely blocked by the main thread",
                MANAGED_NAMING_TIMEOUT.as_millis()
            )),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[inline(never)]
    fn spin_until(flag: &AtomicBool) {
        let mut counter = 0u64;
        while !flag.load(Ordering::Relaxed) {
            counter = std::hint::black_box(counter.wrapping_add(1));
        }
    }

    #[test]
    fn captures_a_busy_thread_stack_through_native_frames() {
        static STOP: AtomicBool = AtomicBool::new(false);
        let (tid_tx, tid_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            tid_tx.send(unsafe { GetCurrentThreadId() }).unwrap();
            spin_until(&STOP);
        });
        let tid = tid_rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let result = capture(
            tid,
            &CaptureRequest {
                samples: Some(2),
                interval_ms: Some(20),
                max_frames: Some(64),
                resolve_managed: Some(false),
            },
            1,
            None,
        )
        .expect("capture");
        STOP.store(true, Ordering::Relaxed);
        worker.join().unwrap();

        assert_eq!(result.samples.len(), 2);
        assert_eq!(result.managed_status, "skipped");
        let function_begin = |address: u64| -> Option<u64> {
            let mut image_base = 0u64;
            let entry = unsafe { RtlLookupFunctionEntry(address, &mut image_base, null_mut()) };
            (!entry.is_null()).then(|| image_base + u64::from(unsafe { (*entry).begin }))
        };
        let spin_begin = function_begin(spin_until as *const () as usize as u64).expect("spin_until unwind data");
        for sample in &result.samples {
            assert!(sample.error.is_none(), "{:?}", sample.error);
            assert!(sample.stack_bytes > 0);
            // The walk must get past the spinning frame into its callers.
            assert!(sample.frames.len() >= 3, "frames: {:?} stop: {:?}", sample.frames, sample.unwind_stop);
            let sps: Vec<u64> = sample.frames.iter().map(|frame| frame.sp).collect();
            assert!(sps.windows(2).all(|pair| pair[0] < pair[1]), "{sps:?}");
            let caller_index = sample
                .frames
                .iter()
                .take(6)
                .position(|frame| function_begin(frame.ip.saturating_sub(1)) == Some(spin_begin)
                    || function_begin(frame.ip) == Some(spin_begin))
                .expect("spin_until frame near the top");
            assert!(sample.frames.len() > caller_index + 1, "walk stopped at the spinning frame");
        }
        assert!(result.cpu_end_100ns >= result.cpu_start_100ns);
    }

    #[test]
    fn name_cache_skips_known_addresses_until_the_generation_changes() {
        let generation = 0x5EED_0001;
        let frame = ManagedFrame {
            ip: 0x10,
            method: "User:Loop ()".to_string(),
            assembly: Some("Assembly-CSharp".to_string()),
            file: None,
            line: None,
            native_offset: 4,
        };
        let (known, missing) = cached_names(generation, vec![(0x10, 0x10), (0x20, 0x1F)]);
        assert!(known.is_empty());
        assert_eq!(missing.len(), 2);
        // 0x20 is native: remembered as a miss so Mono is not asked again.
        remember_names(generation, &missing, std::slice::from_ref(&frame));

        let (known, missing) = cached_names(generation, vec![(0x10, 0x10), (0x20, 0x1F)]);
        assert!(missing.is_empty());
        assert_eq!(known.len(), 1);
        assert_eq!(known[0].method, "User:Loop ()");

        // A domain reload frees JIT code: nothing carries over.
        let (known, missing) = cached_names(generation + 1, vec![(0x10, 0x10)]);
        assert!(known.is_empty());
        assert_eq!(missing, vec![(0x10, 0x10)]);
    }

    #[test]
    fn refuses_to_capture_the_calling_thread() {
        let tid = unsafe { GetCurrentThreadId() };
        assert!(capture(tid, &CaptureRequest::default(), 1, None).is_err());
    }

    #[test]
    fn stack_view_reads_are_bounds_checked() {
        let bytes = 7u64.to_le_bytes().repeat(2);
        let view = StackView {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(view.read(0x1000), Some(7));
        assert_eq!(view.read(0x1008), Some(7));
        assert_eq!(view.read(0x1009), None);
        assert_eq!(view.read(0x0FF8), None);
    }
}
