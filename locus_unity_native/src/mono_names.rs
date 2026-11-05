//! Mono frame naming for main-thread stack captures in the Unity editor.
//! Plugged into the broker as a [`ManagedNamer`]; the generic capture runs the
//! lookup on a helper thread with a deadline.

use std::ffi::{c_char, c_void, CStr};
use std::sync::OnceLock;

use locus_native_broker::hang_stack_types::{ManagedFrame, ManagedNamer};

type Handle = *mut c_void;

unsafe extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const c_char) -> *mut c_void;
}

pub struct MonoNamer;

pub static MONO_NAMER: MonoNamer = MonoNamer;

impl ManagedNamer for MonoNamer {
    fn prepare(&self) {
        let _ = mono_api();
    }

    fn unavailable_reason(&self) -> Option<&'static str> {
        mono_api().is_none().then_some("Mono runtime exports were not found")
    }

    unsafe fn name(&self, lookups: &[(u64, u64)]) -> Result<Vec<ManagedFrame>, String> {
        name_with_mono(lookups)
    }
}

#[repr(C)]
struct MonoDebugSourceLocation {
    source_file: *mut c_char,
    row: u32,
    column: u32,
    il_offset: u32,
}

type DomainFunc = extern "C" fn(*mut c_void, *mut c_void);

struct MonoApi {
    get_root_domain: unsafe extern "C" fn() -> *mut c_void,
    thread_attach: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    thread_detach: unsafe extern "C" fn(*mut c_void),
    domain_foreach: unsafe extern "C" fn(DomainFunc, *mut c_void),
    jit_info_table_find: unsafe extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void,
    jit_info_get_method: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    jit_info_get_code_start: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    method_full_name: unsafe extern "C" fn(*mut c_void, i32) -> *mut c_char,
    method_get_class: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    class_get_image: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    image_get_name: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    free: unsafe extern "C" fn(*mut c_void),
    debug_lookup_source_location:
        Option<unsafe extern "C" fn(*mut c_void, u32, *mut c_void) -> *mut MonoDebugSourceLocation>,
    debug_free_source_location: Option<unsafe extern "C" fn(*mut MonoDebugSourceLocation)>,
}

fn mono_api() -> Option<&'static MonoApi> {
    static API: OnceLock<Option<MonoApi>> = OnceLock::new();
    API.get_or_init(load_mono_api).as_ref()
}

fn load_mono_api() -> Option<MonoApi> {
    const MODULES: &[&str] = &[
        "mono-2.0-bdwgc.dll",
        "mono-2.0-sgen.dll",
        "mono-2.0.dll",
        "mono.dll",
    ];
    let module = MODULES.iter().find_map(|name| {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let handle = unsafe { GetModuleHandleW(wide.as_ptr()) };
        (!handle.is_null()).then_some(handle)
    })?;
    let symbol = |name: &str| -> Option<*mut c_void> {
        let cname = std::ffi::CString::new(name).ok()?;
        let address = unsafe { GetProcAddress(module, cname.as_ptr()) };
        (!address.is_null()).then_some(address)
    };
    macro_rules! required {
        ($name:literal) => {
            unsafe { std::mem::transmute(symbol($name)?) }
        };
    }
    macro_rules! optional {
        ($name:literal) => {
            symbol($name).map(|address| unsafe { std::mem::transmute(address) })
        };
    }
    Some(MonoApi {
        get_root_domain: required!("mono_get_root_domain"),
        thread_attach: required!("mono_thread_attach"),
        thread_detach: required!("mono_thread_detach"),
        domain_foreach: required!("mono_domain_foreach"),
        jit_info_table_find: required!("mono_jit_info_table_find"),
        jit_info_get_method: required!("mono_jit_info_get_method"),
        jit_info_get_code_start: required!("mono_jit_info_get_code_start"),
        method_full_name: required!("mono_method_full_name"),
        method_get_class: required!("mono_method_get_class"),
        class_get_image: required!("mono_class_get_image"),
        image_get_name: required!("mono_image_get_name"),
        free: required!("mono_free"),
        debug_lookup_source_location: optional!("mono_debug_lookup_source_location"),
        debug_free_source_location: optional!("mono_debug_free_source_location"),
    })
}

extern "C" fn collect_domain(domain: *mut c_void, user_data: *mut c_void) {
    unsafe {
        let domains = &mut *(user_data as *mut Vec<usize>);
        if domains.len() < domains.capacity() {
            domains.push(domain as usize);
        }
    }
}

unsafe fn owned_c_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let value = CStr::from_ptr(ptr).to_string_lossy().trim().to_string();
    (!value.is_empty()).then_some(value)
}

/// # Safety
/// Must run on a dedicated thread: it attaches that thread to Mono.
pub(crate) unsafe fn name_with_mono(lookups: &[(u64, u64)]) -> Result<Vec<ManagedFrame>, String> {
    let api = mono_api().ok_or("Mono runtime exports were not found")?;
    let root = (api.get_root_domain)();
    if root.is_null() {
        return Err("Mono root domain is not available".to_string());
    }
    let attached = (api.thread_attach)(root);
    let mut domains: Vec<usize> = Vec::with_capacity(32);
    (api.domain_foreach)(collect_domain, &mut domains as *mut Vec<usize> as *mut c_void);
    // Editor scripts run in the child domain; search it before the root.
    domains.sort_by_key(|domain| *domain == root as usize);
    if domains.is_empty() {
        domains.push(root as usize);
    }

    let mut frames = Vec::new();
    for &(ip, lookup) in lookups {
        for &domain in &domains {
            let domain = domain as *mut c_void;
            let info = (api.jit_info_table_find)(domain, lookup as *mut c_void);
            if info.is_null() {
                continue;
            }
            let method = (api.jit_info_get_method)(info);
            if method.is_null() {
                break;
            }
            let name_ptr = (api.method_full_name)(method, 1);
            let name = owned_c_string(name_ptr);
            if !name_ptr.is_null() {
                (api.free)(name_ptr as *mut c_void);
            }
            let Some(method_name) = name else { break };
            let code_start = (api.jit_info_get_code_start)(info) as u64;
            let native_offset = ip.saturating_sub(code_start);
            let lookup_offset = lookup.saturating_sub(code_start).min(u64::from(u32::MAX)) as u32;
            let class = (api.method_get_class)(method);
            let assembly = if class.is_null() {
                None
            } else {
                let image = (api.class_get_image)(class);
                if image.is_null() {
                    None
                } else {
                    owned_c_string((api.image_get_name)(image))
                }
            };
            let (mut file, mut line) = (None, None);
            if let (Some(lookup_location), Some(free_location)) =
                (api.debug_lookup_source_location, api.debug_free_source_location)
            {
                let location = lookup_location(method, lookup_offset, domain);
                if !location.is_null() {
                    file = owned_c_string((*location).source_file);
                    line = ((*location).row > 0).then_some((*location).row);
                    free_location(location);
                }
            }
            frames.push(ManagedFrame {
                ip,
                method: method_name,
                assembly,
                file,
                line,
                native_offset,
            });
            break;
        }
    }

    if !attached.is_null() {
        (api.thread_detach)(attached);
    }
    Ok(frames)
}

