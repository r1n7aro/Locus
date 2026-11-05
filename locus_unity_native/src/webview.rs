//! Unity-thread-only WebView2 host. Native callbacks survive managed reload.
use serde_json::{json, Value};
use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    ffi::c_void,
    rc::Rc,
};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::core::Error;
use windows::{
    core::*,
    Win32::{
        Foundation::{HWND, POINT, RECT},
        Graphics::Gdi::{RedrawWindow, ScreenToClient, RDW_ALLCHILDREN, RDW_INVALIDATE},
        System::{
            Com::{CoInitializeEx, COINIT_APARTMENTTHREADED},
            Threading::{GetCurrentProcessId, GetCurrentThreadId},
        },
        UI::{Input::KeyboardAndMouse::EnableWindow, WindowsAndMessaging::*},
    },
};

const MAX_CHILDREN: usize = 6;
const MAX_MESSAGE: usize = 1024 * 1024;
#[derive(Default)]
struct Child {
    parent: isize,
    host: isize,
    generation: u64,
    controller: Option<ICoreWebView2Controller>,
    web: Option<ICoreWebView2>,
    closed: bool,
    ready: bool,
    visible: bool,
    input_blocked: bool,
    bounds: [i32; 8],
    focus_token: i64,
    process_token: i64,
    navigation_token: i64,
    popup_token: i64,
}
#[derive(Default)]
struct Host {
    thread: u32,
    wake_parent: isize,
    parking: isize,
    environment: Option<ICoreWebView2Environment>,
    root: Option<ICoreWebView2Controller>,
    web: Option<ICoreWebView2>,
    profile: String,
    url: String,
    ready: bool,
    closed: bool,
    error: String,
    version: String,
    messages: VecDeque<String>,
    children: HashMap<i32, Rc<RefCell<Child>>>,
    message_token: i64,
    popup_token: i64,
    process_token: i64,
    navigation_token: i64,
}
thread_local! {static HOST:RefCell<Option<Rc<RefCell<Host>>>>=const{RefCell::new(None)};static COM:RefCell<Option<i32>>=const{RefCell::new(None)};}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
unsafe fn read(p: *const u8, n: i32) -> std::result::Result<String, String> {
    if p.is_null() || n < 0 || n as usize > MAX_MESSAGE {
        return Err("Invalid UTF-8 payload length".into());
    }
    std::str::from_utf8(std::slice::from_raw_parts(p, n as usize))
        .map(str::to_owned)
        .map_err(|e| e.to_string())
}
fn host() -> Option<Rc<RefCell<Host>>> {
    HOST.with(|h| h.borrow().clone())
        .filter(|h| h.borrow().thread == unsafe { GetCurrentThreadId() })
}
unsafe fn own_window(raw: isize) -> bool {
    let mut pid = 0;
    GetWindowThreadProcessId(HWND(raw as *mut c_void), Some(&mut pid));
    raw != 0 && pid == GetCurrentProcessId()
}
unsafe fn wake(raw: isize) {
    if own_window(raw) {
        let _ = RedrawWindow(
            Some(HWND(raw as *mut c_void)),
            None,
            None,
            RDW_INVALIDATE | RDW_ALLCHILDREN,
        );
    }
}
fn enqueue(h: &Rc<RefCell<Host>>, value: Value) {
    let mut s = h.borrow_mut();
    if s.closed {
        return;
    }
    if s.messages.len() >= 128 {
        s.messages.pop_front();
    }
    s.messages.push_back(value.to_string());
    let parent = s.wake_parent;
    drop(s);
    unsafe {
        wake(parent);
    }
}
fn fail(h: &Rc<RefCell<Host>>, error: impl std::fmt::Display) {
    h.borrow_mut().error = error.to_string();
    enqueue(h, json!({"type":"host-error","message":error.to_string()}));
}
unsafe fn send(h: &Rc<RefCell<Host>>, value: &Value) -> windows::core::Result<()> {
    let web = h
        .borrow()
        .web
        .clone()
        .ok_or_else(|| Error::from_hresult(HRESULT(-1)))?;
    let s = wide(&value.to_string());
    web.PostWebMessageAsString(PCWSTR(s.as_ptr()))
}
fn valid_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://127.0.0.1:") else {
        return false;
    };
    let Some((port, path)) = rest.split_once('/') else {
        return false;
    };
    port.parse::<u16>().is_ok_and(|p| p > 0)
        && path
            .strip_prefix("unity-view.html#")
            .is_some_and(|token| token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()))
}
unsafe fn restrict_navigation(web: &ICoreWebView2, allowed: String) -> windows::core::Result<i64> {
    let mut token = 0;
    web.add_NavigationStarting(
        &NavigationStartingEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                let mut raw = PWSTR::null();
                args.Uri(&mut raw)?;
                let uri = CoTaskMemPWSTR::from(raw).to_string();
                if uri != allowed && !(allowed.starts_with("about:blank#") && uri == "about:blank")
                {
                    args.SetCancel(true)?;
                }
            }
            Ok(())
        })),
        &mut token,
    )?;
    Ok(token)
}

unsafe fn wire_child(
    h: &Rc<RefCell<Host>>,
    id: i32,
    child: Rc<RefCell<Child>>,
    args: ICoreWebView2NewWindowRequestedEventArgs,
) -> windows::core::Result<()> {
    let environment = h
        .borrow()
        .environment
        .clone()
        .ok_or_else(|| Error::from_hresult(HRESULT(-1)))?;
    let defer = args.GetDeferral()?;
    args.SetHandled(true)?;
    let weak = Rc::downgrade(h);
    let child_state = child.clone();
    let deferred = defer.clone();
    let handler = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
        move |result, controller| {
            let setup = (|| -> windows::core::Result<()> {
                result?;
                let c = controller.ok_or_else(|| Error::from_hresult(HRESULT(-1)))?;
                let Some(h) = weak.upgrade().filter(|h| !h.borrow().closed) else {
                    c.Close()?;
                    return Ok(());
                };
                if child_state.borrow().closed {
                    c.Close()?;
                    return Ok(());
                }
                c.SetIsVisible(false)?;
                c.SetBounds(RECT {
                    left: 0,
                    top: 0,
                    right: 320,
                    bottom: 240,
                })?;
                let web = c.CoreWebView2()?;
                let mut focus_token = 0;
                let weak_host = Rc::downgrade(&h);
                c.add_MoveFocusRequested(
                    &MoveFocusRequestedEventHandler::create(Box::new(move |_, args| {
                        if let (Some(h), Some(args)) = (weak_host.upgrade(), args) {
                            let mut reason = COREWEBVIEW2_MOVE_FOCUS_REASON(0);
                            args.Reason(&mut reason)?;
                            args.SetHandled(true)?;
                            enqueue(&h, json!({"type":"move-focus","id":id,"reason":reason.0}));
                        }
                        Ok(())
                    })),
                    &mut focus_token,
                )?;
                let weak_host = Rc::downgrade(&h);
                let mut process_token = 0;
                web.add_ProcessFailed(&ProcessFailedEventHandler::create(Box::new(move |_, args| {
                    let Some(args) = args else { return Ok(()); };
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND(0);
                    args.ProcessFailedKind(&mut kind)?;
                    if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED {
                        if let Some(h) = weak_host.upgrade() { enqueue(&h, json!({"type":"child-error","id":id,"crashed":true,"message":"View 已中断，正在恢复…"})); }
                    }
                    Ok(())
                })), &mut process_token)?;
                let navigation_token =
                    restrict_navigation(&web, format!("about:blank#locus-native-{id}"))?;
                let mut popup_token = 0;
                web.add_NewWindowRequested(
                    &NewWindowRequestedEventHandler::create(Box::new(|_, args| {
                        if let Some(args) = args {
                            args.SetHandled(true)?;
                        }
                        Ok(())
                    })),
                    &mut popup_token,
                )?;
                {
                    let mut state = child_state.borrow_mut();
                    state.controller = Some(c);
                    state.web = Some(web.clone());
                    state.focus_token = focus_token;
                    state.process_token = process_token;
                    state.navigation_token = navigation_token;
                    state.popup_token = popup_token;
                }
                args.SetNewWindow(&web)?;
                Ok(())
            })();
            if let Err(e) = setup {
                if let Some(h) = weak.upgrade() {
                    enqueue(
                        &h,
                        json!({"type":"child-error","id":id,"message":e.to_string()}),
                    );
                }
            }
            let _ = deferred.Complete();
            Ok(())
        },
    ));
    if let Err(e) =
        environment.CreateCoreWebView2Controller(HWND(child.borrow().host as *mut c_void), &handler)
    {
        let _ = defer.Complete();
        return Err(e);
    }
    Ok(())
}

#[no_mangle]
pub unsafe extern "C" fn locus_webview_start(
    parent: isize,
    p: *const u8,
    n: i32,
    u: *const u8,
    un: i32,
) -> i32 {
    let (Ok(profile), Ok(url)) = (read(p, n), read(u, un)) else {
        return -1;
    };
    if !own_window(parent) || !valid_url(&url) || profile.is_empty() {
        return -2;
    }
    if let Some(h) = host() {
        let s = h.borrow();
        return if s.profile == profile && s.url == url {
            0
        } else {
            -3
        };
    }
    if HOST.with(|h| h.borrow().is_some()) {
        return -4;
    }
    let hr = COM.with(|c| {
        *c.borrow_mut()
            .get_or_insert_with(|| CoInitializeEx(None, COINIT_APARTMENTTHREADED).0)
    });
    if hr < 0 {
        return hr;
    }
    let parking = match CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("STATIC"),
        w!("Locus Vue host"),
        WS_POPUP,
        0,
        0,
        1,
        1,
        None,
        None,
        None,
        None,
    ) {
        Ok(h) => h,
        Err(_) => return -5,
    };
    let mut version = PWSTR::null();
    let _ = GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut version);
    let version = if version.is_null() {
        String::new()
    } else {
        CoTaskMemPWSTR::from(version).to_string()
    };
    let h = Rc::new(RefCell::new(Host {
        thread: GetCurrentThreadId(),
        wake_parent: parent,
        parking: parking.0 as isize,
        profile: profile.clone(),
        url: url.clone(),
        version,
        ..Default::default()
    }));
    HOST.with(|s| s.replace(Some(h.clone())));
    let weak = Rc::downgrade(&h);
    let completed = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
        move |result, env| {
            let Some(h) = weak.upgrade().filter(|h| !h.borrow().closed) else {
                return Ok(());
            };
            let setup = (|| -> windows::core::Result<()> {
                result?;
                let env = env.ok_or_else(|| Error::from_hresult(HRESULT(-1)))?;
                h.borrow_mut().environment = Some(env.clone());
                let weak = Rc::downgrade(&h);
                let ready = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
                    move |result, controller| {
                        let Some(h) = weak.upgrade().filter(|h| !h.borrow().closed) else {
                            if let Some(c) = controller {
                                let _ = c.Close();
                            }
                            return Ok(());
                        };
                        let setup = (|| -> windows::core::Result<()> {
                            result?;
                            let c = controller.ok_or_else(|| Error::from_hresult(HRESULT(-1)))?;
                            c.SetIsVisible(false)?;
                            let web = c.CoreWebView2()?;
                            let weak = Rc::downgrade(&h);
                            let mut message = 0;
                            web.add_WebMessageReceived(
                                &WebMessageReceivedEventHandler::create(Box::new(
                                    move |_, args| {
                                        if let (Some(h), Some(args)) = (weak.upgrade(), args) {
                                            let mut raw = PWSTR::null();
                                            args.TryGetWebMessageAsString(&mut raw)?;
                                            let text = CoTaskMemPWSTR::from(raw).to_string();
                                            if text.len() > MAX_MESSAGE {
                                                return Ok(());
                                            }
                                            if let Ok(value) = serde_json::from_str::<Value>(&text)
                                            {
                                                if value["type"] == "host-ready" {
                                                    h.borrow_mut().ready = true;
                                                }
                                                if value["type"] == "bound" {
                                                    if let Some(child) =
                                                        value["id"].as_i64().and_then(|id| {
                                                            h.borrow()
                                                                .children
                                                                .get(&(id as i32))
                                                                .cloned()
                                                        })
                                                    {
                                                        if value["generation"].as_u64()
                                                            == Some(child.borrow().generation)
                                                        {
                                                            child.borrow_mut().ready = true;
                                                        }
                                                    }
                                                }
                                                enqueue(&h, value);
                                            }
                                        }
                                        Ok(())
                                    },
                                )),
                                &mut message,
                            )?;
                            let weak = Rc::downgrade(&h);
                            let mut popup = 0;
                            web.add_NewWindowRequested(&NewWindowRequestedEventHandler::create(Box::new(move|_,args|{
                        if let Some(args)=args{args.SetHandled(true)?;let mut raw=PWSTR::null();args.Uri(&mut raw)?;let uri=CoTaskMemPWSTR::from(raw).to_string();
                            if let (Some(h),Some(id))=(weak.upgrade(),uri.strip_prefix("about:blank#locus-native-").and_then(|s|s.parse::<i32>().ok())){
                                let child=h.borrow().children.get(&id).cloned();
                                if let Some(child)=child.filter(|c|!c.borrow().closed&&c.borrow().controller.is_none()){
                                    if let Err(e)=wire_child(&h,id,child,args){enqueue(&h,json!({"type":"child-error","id":id,"message":e.to_string()}));}
                                }
                            }
                        }Ok(())
                    })),&mut popup)?;
                            let weak = Rc::downgrade(&h);
                            let mut process = 0;
                            web.add_ProcessFailed(
                                &ProcessFailedEventHandler::create(Box::new(move |_, args| {
                                    let Some(args) = args else { return Ok(()); };
                                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND(0);
                                    args.ProcessFailedKind(&mut kind)?;
                                    if kind != COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED && kind != COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED { return Ok(()); }
                                    if let Some(h) = weak.upgrade() {
                                        h.borrow_mut().error = "View 已中断，正在恢复…".into();
                                        enqueue(&h, json!({"type":"host-error","crashed":true,"message":"View 已中断，正在恢复…"}));
                                    }
                                    Ok(())
                                })),
                                &mut process,
                            )?;
                            let navigation = restrict_navigation(&web, url.clone())?;
                            {
                                let mut s = h.borrow_mut();
                                s.message_token = message;
                                s.popup_token = popup;
                                s.process_token = process;
                                s.navigation_token = navigation;
                                s.root = Some(c);
                                s.web = Some(web.clone());
                            }
                            let url = wide(&url);
                            web.Navigate(PCWSTR(url.as_ptr()))?;
                            Ok(())
                        })();
                        if let Err(e) = setup {
                            fail(&h, e);
                        }
                        Ok(())
                    },
                ));
                env.CreateCoreWebView2Controller(parking, &ready)?;
                Ok(())
            })();
            if let Err(e) = setup {
                fail(&h, e);
            }
            Ok(())
        },
    ));
    let p = wide(&profile);
    if let Err(e) = CreateCoreWebView2EnvironmentWithOptions(
        PCWSTR::null(),
        PCWSTR(p.as_ptr()),
        None,
        &completed,
    ) {
        fail(&h, format!("WebView2 Runtime unavailable: {e}"));
        return -6;
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn locus_webview_bind(
    id: i32,
    generation: u64,
    parent: isize,
    p: *const u8,
    n: i32,
) -> i32 {
    let Some(h) = host() else {
        return -1;
    };
    if !own_window(parent) || id <= 0 {
        return -2;
    }
    let Ok(mut binding) =
        read(p, n).and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()))
    else {
        return -3;
    };
    if !h.borrow().ready {
        return 1;
    }
    let existing = h.borrow().children.get(&id).cloned();
    let child = if let Some(child) = existing {
        child
    } else {
        if h.borrow().children.len() >= MAX_CHILDREN {
            return -4;
        }
        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Locus View"),
            WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            0,
            0,
            1,
            1,
            Some(HWND(parent as *mut c_void)),
            None,
            None,
            None,
        ) {
            Ok(w) => w,
            Err(_) => return -5,
        };
        let child = Rc::new(RefCell::new(Child {
            parent,
            host: hwnd.0 as isize,
            ..Default::default()
        }));
        h.borrow_mut().children.insert(id, child.clone());
        child
    };
    let (old_parent, web) = {
        let mut s = child.borrow_mut();
        if generation <= s.generation {
            return -6;
        }
        s.generation = generation;
        s.ready = false;
        s.input_blocked = true;
        (s.parent, s.web.clone())
    };
    if old_parent != parent {
        if locus_webview_parent(id, parent) != 0 {
            return -7;
        }
    }
    let child_host = child.borrow().host;
    let _ = EnableWindow(HWND(child_host as *mut c_void), false);
    if let Some(web) = web {
        if let Ok(web) = web.cast::<ICoreWebView2_3>() {
            let _ = web.Resume();
        }
    }
    binding["type"] = json!("bind");
    binding["id"] = json!(id);
    binding["generation"] = json!(generation);
    h.borrow_mut().wake_parent = parent;
    match send(&h, &binding) {
        Ok(()) => 0,
        Err(e) => {
            fail(&h, e);
            -8
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_message(p: *const u8, n: i32) -> i32 {
    let Some(h) = host() else {
        return -1;
    };
    let Ok(value) =
        read(p, n).and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e| e.to_string()))
    else {
        return -2;
    };
    if send(&h, &value).is_ok() {
        0
    } else {
        -3
    }
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_rect(
    id: i32,
    x: i32,
    y: i32,
    w: i32,
    height: i32,
    cx: i32,
    cy: i32,
    cw: i32,
    ch: i32,
    visible: i32,
) -> i32 {
    let Some(child) = host().and_then(|h| h.borrow().children.get(&id).cloned()) else {
        return -1;
    };
    let (parent, hwnd, c, ready) = {
        let s = child.borrow();
        (s.parent, s.host, s.controller.clone(), s.ready)
    };
    let Some(c) = c else {
        return 1;
    };
    let show = visible != 0 && ready && w > 0 && height > 0 && cw > 0 && ch > 0;
    if !show {
        let _ = c.SetIsVisible(false);
        let _ = ShowWindow(HWND(hwnd as *mut c_void), SW_HIDE);
        child.borrow_mut().visible = false;
        return 0;
    }
    let bounds = [x, y, w, height, cx, cy, cw, ch];
    if child.borrow().input_blocked {
        let _ = EnableWindow(HWND(hwnd as *mut c_void), true);
        child.borrow_mut().input_blocked = false;
    }
    if child.borrow().visible && child.borrow().bounds == bounds {
        return 0;
    }
    let mut origin = POINT { x: cx, y: cy };
    let _ = ScreenToClient(HWND(parent as *mut c_void), &mut origin);
    let result = (|| -> windows::core::Result<()> {
        SetWindowPos(
            HWND(hwnd as *mut c_void),
            Some(HWND_TOP),
            origin.x,
            origin.y,
            cw,
            ch,
            SWP_NOACTIVATE,
        )?;
        c.SetBounds(RECT {
            left: x - cx,
            top: y - cy,
            right: x - cx + w,
            bottom: y - cy + height,
        })?;
        c.NotifyParentWindowPositionChanged()?;
        c.SetIsVisible(true)?;
        let _ = ShowWindow(HWND(hwnd as *mut c_void), SW_SHOWNA);
        Ok(())
    })();
    if result.is_err() {
        return -2;
    }
    let mut s = child.borrow_mut();
    s.visible = true;
    s.bounds = bounds;
    0
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_parent(id: i32, parent: isize) -> i32 {
    if !own_window(parent) {
        return -2;
    }
    let Some(child) = host().and_then(|h| h.borrow().children.get(&id).cloned()) else {
        return -1;
    };
    let (old_parent, child_host, controller) = {
        let s = child.borrow();
        (s.parent, s.host, s.controller.clone())
    };
    if !IsWindow(Some(HWND(child_host as *mut c_void))).as_bool() {
        return -3;
    }
    if old_parent != parent {
        if let Some(c) = controller {
            // Move the controller through its SDK so browser-side ancestor and
            // composition ownership change with the Unity window. A raw
            // SetParent on its container leaves that ownership implicit.
            let next = match CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Locus View"),
                WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                Some(HWND(parent as *mut c_void)),
                None,
                None,
                None,
            ) {
                Ok(window) => window,
                Err(_) => return -4,
            };
            let _ = c.SetIsVisible(false);
            if c.SetParentWindow(next).is_err() {
                let _ = DestroyWindow(next);
                return -4;
            }
            {
                let mut s = child.borrow_mut();
                s.host = next.0 as isize;
                s.parent = parent;
                s.bounds = [0; 8];
                s.visible = false;
            }
            let _ = c.NotifyParentWindowPositionChanged();
            let _ = DestroyWindow(HWND(child_host as *mut c_void));
            return 0;
        }
        if SetParent(
            HWND(child_host as *mut c_void),
            Some(HWND(parent as *mut c_void)),
        )
        .is_err()
        {
            return -4;
        }
        {
            let mut s = child.borrow_mut();
            s.parent = parent;
            s.bounds = [0; 8];
        }
    }
    0
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_focus(id: i32, reason: i32) -> i32 {
    let c = host()
        .and_then(|h| h.borrow().children.get(&id).cloned())
        .and_then(|s| s.borrow().controller.clone());
    if let Some(c) = c {
        if c.MoveFocus(COREWEBVIEW2_MOVE_FOCUS_REASON(reason.clamp(0, 2)))
            .is_ok()
        {
            return 0;
        }
    }
    -1
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_suspend(id: i32) {
    if let Some(web) = host()
        .and_then(|h| h.borrow().children.get(&id).cloned())
        .and_then(|s| s.borrow().web.clone())
    {
        if let Ok(web) = web.cast::<ICoreWebView2_3>() {
            let _ = web.TrySuspend(&TrySuspendCompletedHandler::create(Box::new(|_, _| Ok(()))));
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_park(id: i32) {
    if let Some(h) = host() {
        let parking = h.borrow().parking;
        let _ = locus_webview_rect(id, 0, 0, 0, 0, 0, 0, 0, 0, 0);
        let _ = locus_webview_parent(id, parking);
    }
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_close(id: i32) {
    let Some(h) = host() else {
        return;
    };
    let child = h.borrow_mut().children.remove(&id);
    let _ = send(&h, &json!({"type":"destroy","id":id}));
    if let Some(child) = child {
        let (web, controller, child_host, process, navigation, popup, focus) = {
            let mut s = child.borrow_mut();
            s.closed = true;
            (
                s.web.take(),
                s.controller.take(),
                s.host,
                s.process_token,
                s.navigation_token,
                s.popup_token,
                s.focus_token,
            )
        };
        if let Some(web) = web {
            let _ = web.remove_ProcessFailed(process);
            let _ = web.remove_NavigationStarting(navigation);
            let _ = web.remove_NewWindowRequested(popup);
        }
        if let Some(c) = controller {
            let _ = c.remove_MoveFocusRequested(focus);
            let _ = c.Close();
        }
        let _ = DestroyWindow(HWND(child_host as *mut c_void));
    }
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_shutdown() {
    let Some(h) = host() else {
        return;
    };
    let ids: Vec<i32> = h.borrow().children.keys().copied().collect();
    for id in ids {
        locus_webview_close(id);
    }
    let (web, root, environment, parking, message, popup, process, navigation) = {
        let mut s = h.borrow_mut();
        s.closed = true;
        s.messages.clear();
        (
            s.web.take(),
            s.root.take(),
            s.environment.take(),
            s.parking,
            s.message_token,
            s.popup_token,
            s.process_token,
            s.navigation_token,
        )
    };
    if let Some(web) = web {
        let _ = web.remove_WebMessageReceived(message);
        let _ = web.remove_NewWindowRequested(popup);
        let _ = web.remove_ProcessFailed(process);
        let _ = web.remove_NavigationStarting(navigation);
    }
    if let Some(c) = root {
        let _ = c.Close();
    }
    drop(environment);
    let _ = DestroyWindow(HWND(parking as *mut c_void));
    HOST.with(|h| h.replace(None));
}
unsafe fn copy(value: &str, p: *mut u8, capacity: i32) -> i32 {
    let n = value.len() as i32;
    if p.is_null() || capacity < n {
        return -n;
    }
    std::ptr::copy_nonoverlapping(value.as_ptr(), p, n as usize);
    n
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_poll(p: *mut u8, capacity: i32) -> i32 {
    let Some(h) = host() else {
        return 0;
    };
    let mut s = h.borrow_mut();
    let Some(value) = s.messages.front() else {
        return 0;
    };
    let n = copy(value, p, capacity);
    if n > 0 {
        s.messages.pop_front();
    }
    n
}
#[no_mangle]
pub unsafe extern "C" fn locus_webview_status(p: *mut u8, capacity: i32) -> i32 {
    let value = if let Some(h) = host() {
        let (web, environment, mut snapshot) = {
            let s = h.borrow();
            let children:Vec<_>=s.children.iter().map(|(id,c)|{let c=c.borrow();json!({"id":id,"generation":c.generation,"host":c.host,"parent":c.parent,"ready":c.ready,"visible":c.visible,"bounds":c.bounds})}).collect();
            (
                s.web.clone(),
                s.environment.clone(),
                json!({"running":true,"ready":s.ready,"error":s.error,"runtimeVersion":s.version,"children":children,"queuedMessages":s.messages.len()}),
            )
        };
        let mut browser_pid = 0;
        if let Some(web) = web {
            let _ = web.BrowserProcessId(&mut browser_pid);
        }
        let mut processes = Vec::new();
        if let Some(env) = environment
            .as_ref()
            .and_then(|e| e.cast::<ICoreWebView2Environment8>().ok())
        {
            if let Ok(infos) = env.GetProcessInfos() {
                let mut count = 0;
                let _ = infos.Count(&mut count);
                for index in 0..count {
                    if let Ok(info) = infos.GetValueAtIndex(index) {
                        let mut pid = 0;
                        let mut kind = COREWEBVIEW2_PROCESS_KIND(0);
                        let _ = info.ProcessId(&mut pid);
                        let _ = info.Kind(&mut kind);
                        processes.push(json!({"pid":pid,"kind":kind.0}));
                    }
                }
            }
        }
        snapshot["browserProcessId"] = json!(browser_pid);
        snapshot["processes"] = json!(processes);
        snapshot
    } else {
        json!({"running":false,"ready":false,"children":[]})
    };
    copy(&value.to_string(), p, capacity)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_loopback_bootstrap() {
        assert!(valid_url(&format!(
            "http://127.0.0.1:1234/unity-view.html#{}",
            "a".repeat(64)
        )));
        for url in [
            "http://evil.test/unity-view.html#t",
            "http://127.0.0.1:0/unity-view.html#t",
            "http://127.0.0.1:1234/other#t",
            "http://127.0.0.1:1234/unity-view.html#short",
        ] {
            assert!(!valid_url(url));
        }
    }
}
