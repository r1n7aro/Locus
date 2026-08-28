use serde::{Deserialize, Serialize};

pub const UNITY_MODAL_DIALOG_BLOCKED_CODE: &str = "unity_modal_dialog_blocked";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnityDialogChoice {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnityModalDialog {
    pub code: String,
    pub dialog_id: String,
    pub project: String,
    pub title: String,
    pub message: String,
    pub choices: Vec<UnityDialogChoice>,
    pub main_thread_blocked: bool,
    pub opened_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UnityDialogChoiceResult {
    pub dialog_id: String,
    pub choice_id: String,
    pub label: String,
    pub invoked: bool,
}

/// Native-broker messages that can remain usable while Unity's managed main
/// thread is inside a modal message loop. Everything else is conservatively
/// classified as a main-thread request.
pub fn message_requires_unity_main_thread(message_type: &str) -> bool {
    !matches!(
        message_type,
        "ping"
            | "status"
            | "bridge_capabilities"
            | "cancel_execute_code"
            | "execute_code_progress"
            | "get_reload_state"
            | "get_compile_result"
    )
}

pub fn is_unity_modal_dialog_blocked_error(error: &str) -> bool {
    error.contains(UNITY_MODAL_DIALOG_BLOCKED_CODE)
}

#[cfg(windows)]
mod platform {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::{
        collections::{HashMap, HashSet},
        ffi::c_void,
        path::Path,
        sync::{
            atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering},
            mpsc, Arc, Mutex, MutexGuard, OnceLock,
        },
        thread,
        time::Duration,
    };
    use tokio::sync::watch;
    use windows::{
        core::BOOL,
        Win32::{
            Foundation::{HWND, LPARAM, RECT, WPARAM},
            System::{
                Com::{
                    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
                    COINIT_MULTITHREADED,
                },
                Threading::GetCurrentThreadId,
            },
            UI::{
                Accessibility::{
                    CUIAutomation, IUIAutomation, IUIAutomationInvokePattern, SetWinEventHook,
                    TreeScope_Descendants, UIA_ButtonControlTypeId, UIA_InvokePatternId,
                    UIA_TextControlTypeId, UnhookWinEvent, HWINEVENTHOOK,
                },
                Input::KeyboardAndMouse::IsWindowEnabled,
                WindowsAndMessaging::{
                    DispatchMessageW, EnumChildWindows, EnumWindows, GetAncestor, GetClassNameW,
                    GetMessageW, GetWindow, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
                    GetWindowThreadProcessId, IsWindow, IsWindowVisible, PostMessageW,
                    PostThreadMessageW, SendMessageW, TranslateMessage, BM_CLICK, CHILDID_SELF,
                    EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
                    EVENT_OBJECT_STATECHANGE, EVENT_SYSTEM_DIALOGEND, EVENT_SYSTEM_DIALOGSTART,
                    EVENT_SYSTEM_FOREGROUND, GA_ROOT, GW_OWNER, MSG, OBJID_WINDOW,
                    WINEVENT_OUTOFCONTEXT, WM_GETTEXT, WM_GETTEXTLENGTH, WM_QUIT,
                },
            },
        },
    };

    const HOOK_READY_TIMEOUT: Duration = Duration::from_secs(2);
    const MAX_UIA_ELEMENTS: i32 = 512;

    #[derive(Debug)]
    struct HookEntry {
        project_key: String,
        project_path: String,
        process_id: u32,
        process_created_at_ms: u64,
        main_hwnd: AtomicIsize,
        hook_thread_id: AtomicU32,
        stopped: AtomicBool,
    }

    impl HookEntry {
        fn identity_matches(&self, process_id: u32, process_created_at_ms: u64) -> bool {
            self.process_id == process_id
                && self.process_created_at_ms == process_created_at_ms
                && !self.stopped.load(Ordering::Acquire)
        }

        fn stop(&self) {
            if self.stopped.swap(true, Ordering::AcqRel) {
                return;
            }
            let thread_id = self.hook_thread_id.load(Ordering::Acquire);
            if thread_id != 0 {
                let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
            }
        }
    }

    #[derive(Debug, Clone, Copy)]
    struct NativeWindowEvent {
        event: u32,
        hwnd: isize,
        id_object: i32,
        id_child: i32,
    }

    #[derive(Debug, Clone)]
    struct NativeChoice {
        public: UnityDialogChoice,
        native_hwnd: isize,
        ordinal: usize,
    }

    #[derive(Debug, Clone)]
    struct DialogRecord {
        public: UnityModalDialog,
        project_key: String,
        process_id: u32,
        process_created_at_ms: u64,
        main_hwnd: isize,
        dialog_hwnd: isize,
        fingerprint: String,
        choices: Vec<NativeChoice>,
        consumed: bool,
    }

    #[derive(Debug)]
    struct HookRoute {
        sender: mpsc::Sender<NativeWindowEvent>,
    }

    fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn hooks() -> &'static Mutex<HashMap<String, Arc<HookEntry>>> {
        static HOOKS: OnceLock<Mutex<HashMap<String, Arc<HookEntry>>>> = OnceLock::new();
        HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn hook_routes() -> &'static Mutex<HashMap<usize, HookRoute>> {
        static ROUTES: OnceLock<Mutex<HashMap<usize, HookRoute>>> = OnceLock::new();
        ROUTES.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn dialogs() -> &'static Mutex<HashMap<String, DialogRecord>> {
        static DIALOGS: OnceLock<Mutex<HashMap<String, DialogRecord>>> = OnceLock::new();
        DIALOGS.get_or_init(|| Mutex::new(HashMap::new()))
    }

    fn dialog_revision_sender() -> &'static watch::Sender<u64> {
        static SENDER: OnceLock<watch::Sender<u64>> = OnceLock::new();
        SENDER.get_or_init(|| watch::channel(0).0)
    }

    fn notify_dialog_revision() {
        static REVISION: AtomicU64 = AtomicU64::new(0);
        let sender = dialog_revision_sender();
        let next = REVISION.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        sender.send_replace(next);
    }

    pub fn subscribe() -> watch::Receiver<u64> {
        dialog_revision_sender().subscribe()
    }

    fn project_key(project_path: &str) -> String {
        let trimmed = project_path.trim().trim_matches('"');
        let path =
            dunce::canonicalize(trimmed).unwrap_or_else(|_| Path::new(trimmed).to_path_buf());
        let mut value = path.to_string_lossy().replace('/', "\\");
        while value.len() > 3 && value.ends_with('\\') {
            value.pop();
        }
        value.to_ascii_lowercase()
    }

    fn normalized_project_path(project_path: &str) -> String {
        let trimmed = project_path.trim().trim_matches('"');
        let path =
            dunce::canonicalize(trimmed).unwrap_or_else(|_| Path::new(trimmed).to_path_buf());
        let mut value = path.to_string_lossy().replace('/', "\\");
        while value.len() > 3 && value.ends_with('\\') {
            value.pop();
        }
        value
    }

    unsafe extern "system" fn win_event_callback(
        hook: HWINEVENTHOOK,
        event: u32,
        hwnd: HWND,
        id_object: i32,
        id_child: i32,
        _event_thread: u32,
        _event_time: u32,
    ) {
        if hwnd.0.is_null() {
            return;
        }
        if event >= EVENT_OBJECT_CREATE
            && (id_object != OBJID_WINDOW.0 || id_child != CHILDID_SELF as i32)
        {
            return;
        }
        let sender = lock_unpoisoned(hook_routes())
            .get(&(hook.0 as usize))
            .map(|route| route.sender.clone());
        if let Some(sender) = sender {
            let _ = sender.send(NativeWindowEvent {
                event,
                hwnd: hwnd.0 as isize,
                id_object,
                id_child,
            });
        }
    }

    pub async fn ensure_project_observed(project_path: &str) -> Result<(), String> {
        let process = super::super::query_current_project_editor_process(project_path).await;
        let Some(process_id) = process.process_id else {
            sync_project_process(project_path, None, None).await;
            return Ok(());
        };
        let process_created_at_ms = super::super::process::process_created_at_unix_ms(process_id)
            .ok_or_else(|| {
            format!("Could not read Unity process {process_id} creation time")
        })?;
        sync_project_process(project_path, Some(process_id), Some(process_created_at_ms)).await;
        Ok(())
    }

    pub async fn sync_project_process(
        project_path: &str,
        process_id: Option<u32>,
        process_created_at_ms: Option<u64>,
    ) {
        let key = project_key(project_path);
        let Some(process_id) = process_id else {
            clear_project_by_key(&key);
            return;
        };
        let Some(process_created_at_ms) = process_created_at_ms else {
            return;
        };

        let existing_matches = lock_unpoisoned(hooks())
            .get(&key)
            .map(|entry| entry.identity_matches(process_id, process_created_at_ms))
            .unwrap_or(false);
        if existing_matches {
            return;
        }

        let project_path = normalized_project_path(project_path);
        let key_for_task = key.clone();
        let result = tokio::task::spawn_blocking(move || {
            install_project_hook(
                key_for_task,
                project_path,
                process_id,
                process_created_at_ms,
            )
        })
        .await;
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("[Locus] Unity modal dialog hook unavailable: {error}"),
            Err(error) => eprintln!("[Locus] Unity modal dialog hook task failed: {error}"),
        }
    }

    fn install_project_hook(
        key: String,
        project_path: String,
        process_id: u32,
        process_created_at_ms: u64,
    ) -> Result<(), String> {
        let entry = Arc::new(HookEntry {
            project_key: key.clone(),
            project_path,
            process_id,
            process_created_at_ms,
            main_hwnd: AtomicIsize::new(0),
            hook_thread_id: AtomicU32::new(0),
            stopped: AtomicBool::new(false),
        });

        {
            let mut map = lock_unpoisoned(hooks());
            if let Some(existing) = map.get(&key) {
                if existing.identity_matches(process_id, process_created_at_ms) {
                    return Ok(());
                }
                existing.stop();
            }
            map.insert(key.clone(), entry.clone());
        }
        clear_dialog_by_key(&key);

        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name(format!("locus-unity-dialog-hook-{process_id}"))
            .spawn(move || hook_thread(entry, ready_tx))
            .map_err(|error| format!("Could not spawn WinEventHook thread: {error}"))?;

        match ready_rx.recv_timeout(HOOK_READY_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Err(format!(
                "WinEventHook for Unity PID {process_id} did not initialize within {}ms",
                HOOK_READY_TIMEOUT.as_millis()
            )),
        }
    }

    fn hook_thread(entry: Arc<HookEntry>, ready_tx: mpsc::SyncSender<Result<(), String>>) {
        entry
            .hook_thread_id
            .store(unsafe { GetCurrentThreadId() }, Ordering::Release);
        let (event_tx, event_rx) = mpsc::channel();

        let system_hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_DIALOGEND,
                None,
                Some(win_event_callback),
                entry.process_id,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };
        let object_hook = unsafe {
            SetWinEventHook(
                EVENT_OBJECT_DESTROY,
                EVENT_OBJECT_STATECHANGE,
                None,
                Some(win_event_callback),
                entry.process_id,
                0,
                WINEVENT_OUTOFCONTEXT,
            )
        };

        if system_hook.0.is_null() || object_hook.0.is_null() {
            if !system_hook.0.is_null() {
                let _ = unsafe { UnhookWinEvent(system_hook) };
            }
            if !object_hook.0.is_null() {
                let _ = unsafe { UnhookWinEvent(object_hook) };
            }
            let _ = ready_tx.send(Err(format!(
                "SetWinEventHook failed for Unity PID {}",
                entry.process_id
            )));
            remove_hook_if_same(&entry);
            return;
        }

        {
            let mut routes = lock_unpoisoned(hook_routes());
            routes.insert(
                system_hook.0 as usize,
                HookRoute {
                    sender: event_tx.clone(),
                },
            );
            routes.insert(
                object_hook.0 as usize,
                HookRoute {
                    sender: event_tx.clone(),
                },
            );
        }

        let worker_entry = entry.clone();
        let (worker_ready_tx, worker_ready_rx) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name(format!("locus-unity-dialog-worker-{}", entry.process_id))
            .spawn(move || dialog_worker(worker_entry, event_rx, worker_ready_tx));
        if let Err(error) = worker {
            lock_unpoisoned(hook_routes()).remove(&(system_hook.0 as usize));
            lock_unpoisoned(hook_routes()).remove(&(object_hook.0 as usize));
            unsafe {
                let _ = UnhookWinEvent(system_hook);
                let _ = UnhookWinEvent(object_hook);
            }
            let _ = ready_tx.send(Err(format!("Could not spawn dialog worker: {error}")));
            remove_hook_if_same(&entry);
            return;
        }

        if worker_ready_rx.recv_timeout(HOOK_READY_TIMEOUT).is_err() {
            lock_unpoisoned(hook_routes()).remove(&(system_hook.0 as usize));
            lock_unpoisoned(hook_routes()).remove(&(object_hook.0 as usize));
            unsafe {
                let _ = UnhookWinEvent(system_hook);
                let _ = UnhookWinEvent(object_hook);
            }
            let _ = ready_tx.send(Err(format!(
                "Initial Unity dialog snapshot timed out for PID {}",
                entry.process_id
            )));
            entry.stop();
            remove_hook_if_same(&entry);
            return;
        }

        let _ = ready_tx.send(Ok(()));

        let mut message = MSG::default();
        loop {
            let result = unsafe { GetMessageW(&mut message, None, 0, 0) };
            if result.0 <= 0 {
                break;
            }
            unsafe {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        lock_unpoisoned(hook_routes()).remove(&(system_hook.0 as usize));
        lock_unpoisoned(hook_routes()).remove(&(object_hook.0 as usize));
        unsafe {
            let _ = UnhookWinEvent(system_hook);
            let _ = UnhookWinEvent(object_hook);
        }
        entry.stopped.store(true, Ordering::Release);
        drop(event_tx);
        clear_dialog_if_identity(&entry);
        remove_hook_if_same(&entry);
    }

    fn remove_hook_if_same(entry: &Arc<HookEntry>) {
        let mut map = lock_unpoisoned(hooks());
        if map
            .get(&entry.project_key)
            .map(|current| Arc::ptr_eq(current, entry))
            .unwrap_or(false)
        {
            map.remove(&entry.project_key);
        }
    }

    fn clear_project_by_key(key: &str) {
        let entry = {
            let mut entries = lock_unpoisoned(hooks());
            entries.remove(key)
        };
        if let Some(entry) = entry {
            entry.stop();
        }
        clear_dialog_by_key(key);
    }

    fn clear_dialog_by_key(key: &str) {
        let removed = {
            let mut records = lock_unpoisoned(dialogs());
            records.remove(key).is_some()
        };
        if removed {
            notify_dialog_revision();
        }
    }

    fn clear_dialog_if_identity(entry: &HookEntry) {
        let mut records = lock_unpoisoned(dialogs());
        let should_remove = records
            .get(&entry.project_key)
            .map(|record| {
                record.process_id == entry.process_id
                    && record.process_created_at_ms == entry.process_created_at_ms
            })
            .unwrap_or(false);
        if should_remove {
            records.remove(&entry.project_key);
            drop(records);
            notify_dialog_revision();
        }
    }

    fn dialog_worker(
        entry: Arc<HookEntry>,
        receiver: mpsc::Receiver<NativeWindowEvent>,
        ready: mpsc::SyncSender<()>,
    ) {
        let com_initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        let automation: Option<IUIAutomation> = if com_initialized {
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()
        } else {
            None
        };

        process_window_event(
            &entry,
            NativeWindowEvent {
                event: EVENT_SYSTEM_DIALOGSTART,
                hwnd: 0,
                id_object: OBJID_WINDOW.0,
                id_child: CHILDID_SELF as i32,
            },
            automation.as_ref(),
        );
        let _ = ready.send(());

        while let Ok(event) = receiver.recv() {
            if entry.stopped.load(Ordering::Acquire) {
                break;
            }
            if super::super::process::process_created_at_unix_ms(entry.process_id)
                != Some(entry.process_created_at_ms)
            {
                entry.stop();
                break;
            }
            process_window_event(&entry, event, automation.as_ref());
        }

        if com_initialized {
            unsafe { CoUninitialize() };
        }
    }

    fn process_window_event(
        entry: &HookEntry,
        event: NativeWindowEvent,
        automation: Option<&IUIAutomation>,
    ) {
        let _ = (event.id_object, event.id_child);
        if matches!(
            event.event,
            EVENT_OBJECT_DESTROY | EVENT_OBJECT_HIDE | EVENT_SYSTEM_DIALOGEND
        ) {
            let root = root_window(event.hwnd);
            let should_clear = {
                let records = lock_unpoisoned(dialogs());
                records
                    .get(&entry.project_key)
                    .map(|record| event.hwnd == record.dialog_hwnd || root == record.dialog_hwnd)
                    .unwrap_or(false)
            };
            if should_clear {
                clear_dialog_by_key(&entry.project_key);
            }
        }

        let mut main_hwnd = entry.main_hwnd.load(Ordering::Acquire);
        if !window_matches_process(main_hwnd, entry.process_id) {
            main_hwnd = find_unity_main_window(entry.process_id).unwrap_or(0);
            entry.main_hwnd.store(main_hwnd, Ordering::Release);
        }
        if main_hwnd == 0 {
            return;
        }

        let mut candidates = Vec::new();
        let event_root = root_window(event.hwnd);
        if event_root != 0 && event_root != main_hwnd {
            candidates.push(event_root);
        }
        if event.hwnd == 0 || event_root == main_hwnd || candidates.is_empty() {
            candidates.extend(owned_top_level_windows(entry.process_id, main_hwnd));
        }
        candidates.sort_unstable();
        candidates.dedup();

        for dialog_hwnd in candidates {
            if let Some(record) = inspect_dialog(entry, main_hwnd, dialog_hwnd, automation) {
                publish_dialog(record);
                return;
            }
        }

        let current = {
            let records = lock_unpoisoned(dialogs());
            records.get(&entry.project_key).cloned()
        };
        if let Some(record) = current {
            if !record_still_valid(&record) {
                clear_dialog_by_key(&entry.project_key);
            }
        }
    }

    fn publish_dialog(mut record: DialogRecord) {
        let mut records = lock_unpoisoned(dialogs());
        let changed = match records.get(&record.project_key) {
            Some(existing)
                if existing.dialog_hwnd == record.dialog_hwnd
                    && existing.fingerprint == record.fingerprint =>
            {
                record.public.dialog_id = existing.public.dialog_id.clone();
                record.public.opened_at_ms = existing.public.opened_at_ms;
                record.consumed = existing.consumed;
                false
            }
            _ => true,
        };
        records.insert(record.project_key.clone(), record);
        drop(records);
        if changed {
            notify_dialog_revision();
        }
    }

    fn inspect_dialog(
        entry: &HookEntry,
        main_hwnd: isize,
        dialog_hwnd: isize,
        automation: Option<&IUIAutomation>,
    ) -> Option<DialogRecord> {
        if dialog_hwnd == 0
            || dialog_hwnd == main_hwnd
            || !window_matches_process(dialog_hwnd, entry.process_id)
            || !window_visible(dialog_hwnd)
            || window_enabled(main_hwnd)
            || !owner_chain_reaches(dialog_hwnd, main_hwnd)
        {
            return None;
        }

        let title = window_text(dialog_hwnd);
        let class_name = window_class(dialog_hwnd);
        let (mut message, mut choices) = automation
            .and_then(|uia| inspect_with_uia(uia, dialog_hwnd, &title).ok())
            .unwrap_or_else(|| inspect_win32_children(dialog_hwnd, &title));
        if message.trim().is_empty() || choices.is_empty() {
            let (win32_message, win32_choices) = inspect_win32_children(dialog_hwnd, &title);
            if message.trim().is_empty() {
                message = win32_message;
            }
            if choices.is_empty() {
                choices = win32_choices;
            }
        }
        if choices.is_empty() {
            return None;
        }

        for (index, choice) in choices.iter_mut().enumerate() {
            choice.public.id = format!("choice-{index}");
            choice.ordinal = index;
        }
        let fingerprint = dialog_fingerprint(&title, &message, &class_name, &choices);
        let public_choices = choices.iter().map(|choice| choice.public.clone()).collect();

        Some(DialogRecord {
            public: UnityModalDialog {
                code: UNITY_MODAL_DIALOG_BLOCKED_CODE.to_string(),
                dialog_id: format!("dialog-{}", uuid::Uuid::new_v4().simple()),
                project: entry.project_path.clone(),
                title,
                message,
                choices: public_choices,
                main_thread_blocked: true,
                opened_at_ms: super::super::unix_now_ms(),
            },
            project_key: entry.project_key.clone(),
            process_id: entry.process_id,
            process_created_at_ms: entry.process_created_at_ms,
            main_hwnd,
            dialog_hwnd,
            fingerprint,
            choices,
            consumed: false,
        })
    }

    fn inspect_with_uia(
        automation: &IUIAutomation,
        dialog_hwnd: isize,
        title: &str,
    ) -> windows::core::Result<(String, Vec<NativeChoice>)> {
        let root = unsafe { automation.ElementFromHandle(hwnd(dialog_hwnd))? };
        let condition = unsafe { automation.CreateTrueCondition()? };
        let elements = unsafe { root.FindAll(TreeScope_Descendants, &condition)? };
        let length = unsafe { elements.Length()? }.clamp(0, MAX_UIA_ELEMENTS);
        let mut text_parts = Vec::new();
        let mut fallback_parts = Vec::new();
        let mut seen_text = HashSet::new();
        let mut choices = Vec::new();

        for index in 0..length {
            let Ok(element) = (unsafe { elements.GetElement(index) }) else {
                continue;
            };
            let name = unsafe { element.CurrentName() }
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default()
                .trim()
                .to_string();
            if name.is_empty() {
                continue;
            }
            let control_type = unsafe { element.CurrentControlType() }.ok();
            let class_name = unsafe { element.CurrentClassName() }
                .ok()
                .map(|value| value.to_string())
                .unwrap_or_default();
            let native_hwnd = unsafe { element.CurrentNativeWindowHandle() }
                .ok()
                .map(|value| value.0 as isize)
                .unwrap_or(0);
            if control_type == Some(UIA_ButtonControlTypeId)
                || class_name.eq_ignore_ascii_case("Button")
            {
                if native_hwnd == 0 || native_hwnd == dialog_hwnd {
                    continue;
                }
                choices.push(NativeChoice {
                    public: UnityDialogChoice {
                        id: String::new(),
                        label: name,
                    },
                    native_hwnd,
                    ordinal: choices.len(),
                });
                continue;
            }
            if name == title || !seen_text.insert(name.clone()) {
                continue;
            }
            if control_type == Some(UIA_TextControlTypeId)
                || class_name.eq_ignore_ascii_case("Edit")
                || class_name.eq_ignore_ascii_case("Static")
            {
                text_parts.push(name);
            } else {
                fallback_parts.push(name);
            }
        }

        let message = if text_parts.is_empty() {
            fallback_parts.join("\n")
        } else {
            text_parts.join("\n")
        };
        Ok((message, choices))
    }

    #[derive(Default)]
    struct Win32ChildSnapshot {
        message_parts: Vec<String>,
        choices: Vec<NativeChoice>,
    }

    unsafe extern "system" fn enum_child_callback(child: HWND, parameter: LPARAM) -> BOOL {
        let snapshot = &mut *(parameter.0 as *mut Win32ChildSnapshot);
        let class_name = window_class(child.0 as isize);
        let text = control_text(child.0 as isize).trim().to_string();
        if text.is_empty() {
            return BOOL(1);
        }
        if class_name.eq_ignore_ascii_case("Button") {
            snapshot.choices.push(NativeChoice {
                public: UnityDialogChoice {
                    id: String::new(),
                    label: text,
                },
                native_hwnd: child.0 as isize,
                ordinal: snapshot.choices.len(),
            });
        } else if class_name.eq_ignore_ascii_case("Static")
            || class_name.eq_ignore_ascii_case("Edit")
        {
            snapshot.message_parts.push(text);
        }
        BOOL(1)
    }

    fn inspect_win32_children(dialog_hwnd: isize, title: &str) -> (String, Vec<NativeChoice>) {
        let mut snapshot = Win32ChildSnapshot::default();
        unsafe {
            let _ = EnumChildWindows(
                Some(hwnd(dialog_hwnd)),
                Some(enum_child_callback),
                LPARAM((&mut snapshot as *mut Win32ChildSnapshot) as isize),
            );
        }
        snapshot.message_parts.retain(|part| part != title);
        snapshot.message_parts.dedup();
        (snapshot.message_parts.join("\n"), snapshot.choices)
    }

    fn dialog_fingerprint(
        title: &str,
        message: &str,
        class_name: &str,
        choices: &[NativeChoice],
    ) -> String {
        let mut hasher = Sha256::new();
        for part in [title, message, class_name] {
            hasher.update(part.as_bytes());
            hasher.update([0]);
        }
        for choice in choices {
            hasher.update(choice.public.label.as_bytes());
            hasher.update([0]);
        }
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn record_still_valid(record: &DialogRecord) -> bool {
        super::super::process::process_created_at_unix_ms(record.process_id)
            == Some(record.process_created_at_ms)
            && window_matches_process(record.main_hwnd, record.process_id)
            && window_matches_process(record.dialog_hwnd, record.process_id)
            && window_visible(record.dialog_hwnd)
            && !window_enabled(record.main_hwnd)
            && owner_chain_reaches(record.dialog_hwnd, record.main_hwnd)
    }

    pub fn current_dialog(project_path: &str) -> Option<UnityModalDialog> {
        let key = project_key(project_path);
        let record = {
            let records = lock_unpoisoned(dialogs());
            records.get(&key).cloned()
        }?;
        if record_still_valid(&record) {
            Some(record.public)
        } else {
            clear_dialog_by_key(&key);
            None
        }
    }

    pub fn blocked_error(
        project_path: &str,
        request_state: &str,
        request_id: Option<&str>,
    ) -> Option<String> {
        current_dialog(project_path)
            .map(|dialog| format_blocked_error(&dialog, request_state, request_id))
    }

    fn format_blocked_error(
        dialog: &UnityModalDialog,
        request_state: &str,
        request_id: Option<&str>,
    ) -> String {
        let mut lines = vec![
            format!("code={UNITY_MODAL_DIALOG_BLOCKED_CODE}"),
            "Unity 主线程已被模态弹窗阻塞。".to_string(),
            format!("request_state={request_state}"),
            format!("dialog_id={}", dialog.dialog_id),
            format!("title={}", display_or_empty(&dialog.title)),
            format!("message={}", display_or_empty(&dialog.message)),
            "choices:".to_string(),
        ];
        for choice in &dialog.choices {
            lines.push(format!("- {}: {}", choice.id, choice.label));
        }
        if let Some(request_id) = request_id {
            lines.push(format!("request_id={request_id}"));
        }
        lines.push("该恢复接口不使用 Unity 主线程。请根据弹窗语义选择一个 choice_id：".to_string());
        let project_json =
            serde_json::to_string(&dialog.project).unwrap_or_else(|_| "\"\"".to_string());
        let dialog_json =
            serde_json::to_string(&dialog.dialog_id).unwrap_or_else(|_| "\"\"".to_string());
        lines.push(format!(
            "python -c 'import asyncio,locus; asyncio.run(locus.choose_unity_dialog(project={project_json}, dialog_id={dialog_json}, choice_id=\"choice-0\"))'"
        ));
        lines.push(match request_state {
            "not_sent" => "选择后可安全重试原 Unity 操作。".to_string(),
            "detached" => {
                let execution_json = serde_json::to_string(request_id.unwrap_or_default())
                    .unwrap_or_else(|_| "\"\"".to_string());
                format!(
                    "原请求已发送；选择后使用 Python SDK 获取原执行结果，避免重复执行：\npython -c 'import asyncio,locus; print(asyncio.run(locus.wait_unity_execution(project={project_json}, execution_id={execution_json})))'"
                )
            }
            _ => "原请求可能已经发送；选择后先查询原请求状态，避免直接重复执行。".to_string(),
        });
        lines.join("\n")
    }

    fn display_or_empty(value: &str) -> &str {
        if value.trim().is_empty() {
            "<empty>"
        } else {
            value
        }
    }

    pub async fn choose_dialog(
        project_path: &str,
        dialog_id: &str,
        choice_id: &str,
    ) -> Result<UnityDialogChoiceResult, String> {
        let project_path = project_path.to_string();
        let dialog_id = dialog_id.to_string();
        let choice_id = choice_id.to_string();
        tokio::task::spawn_blocking(move || {
            choose_dialog_blocking(&project_path, &dialog_id, &choice_id)
        })
        .await
        .map_err(|error| format!("Unity dialog choice task failed: {error}"))?
    }

    fn choose_dialog_blocking(
        project_path: &str,
        dialog_id: &str,
        choice_id: &str,
    ) -> Result<UnityDialogChoiceResult, String> {
        let key = project_key(project_path);
        let (record, choice) = {
            let mut records = lock_unpoisoned(dialogs());
            let record = records.get_mut(&key).ok_or_else(|| {
                "No blocking Unity dialog is registered for this project".to_string()
            })?;
            if record.public.dialog_id != dialog_id {
                return Err("Unity dialog id is stale or belongs to another project".to_string());
            }
            if record.consumed {
                return Err("Unity dialog choice is already being invoked".to_string());
            }
            let choice = record
                .choices
                .iter()
                .find(|choice| choice.public.id == choice_id)
                .cloned()
                .ok_or_else(|| format!("Unknown Unity dialog choice id '{choice_id}'"))?;
            record.consumed = true;
            (record.clone(), choice)
        };

        let result = invoke_choice(&record, &choice);
        if let Err(error) = result {
            if let Some(current) = lock_unpoisoned(dialogs()).get_mut(&key) {
                if current.public.dialog_id == dialog_id {
                    current.consumed = false;
                }
            }
            return Err(error);
        }

        Ok(UnityDialogChoiceResult {
            dialog_id: dialog_id.to_string(),
            choice_id: choice_id.to_string(),
            label: choice.public.label,
            invoked: true,
        })
    }

    fn invoke_choice(record: &DialogRecord, choice: &NativeChoice) -> Result<(), String> {
        if !record_still_valid(record) {
            return Err("Unity dialog changed or closed before the choice was invoked".to_string());
        }
        if window_text(record.dialog_hwnd) != record.public.title {
            return Err("Unity dialog title changed before the choice was invoked".to_string());
        }
        let class_name = window_class(record.dialog_hwnd);
        let (_, current_choices) = inspect_win32_children(record.dialog_hwnd, &record.public.title);
        let expected_labels = record
            .choices
            .iter()
            .map(|candidate| candidate.public.label.as_str())
            .collect::<Vec<_>>();
        let current_win32_labels = current_choices
            .iter()
            .map(|candidate| candidate.public.label.as_str())
            .collect::<Vec<_>>();
        let (uia_invoked, uia_verified) = validate_and_invoke_choice_with_uia(record, choice)?;
        if uia_invoked {
            return Ok(());
        }
        if !uia_verified
            && !current_win32_labels.is_empty()
            && current_win32_labels != expected_labels
        {
            return Err("Unity dialog buttons changed before the choice was invoked".to_string());
        }

        let win32_choice_still_matches = choice.native_hwnd != 0
            && current_choices.iter().any(|candidate| {
                candidate.native_hwnd == choice.native_hwnd
                    && candidate.public.label == choice.public.label
            });
        if win32_choice_still_matches
            && window_matches_process(choice.native_hwnd, record.process_id)
            && owner_root(choice.native_hwnd) == record.dialog_hwnd
        {
            unsafe {
                PostMessageW(
                    Some(hwnd(choice.native_hwnd)),
                    BM_CLICK,
                    WPARAM(0),
                    LPARAM(0),
                )
            }
            .map_err(|error| format!("Could not invoke Unity dialog button: {error}"))?;
            return Ok(());
        }

        Err(format!(
            "Unity dialog choice changed before invocation (dialog class '{class_name}')"
        ))
    }

    fn validate_and_invoke_choice_with_uia(
        record: &DialogRecord,
        choice: &NativeChoice,
    ) -> Result<(bool, bool), String> {
        let com_initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
        if !com_initialized {
            return Ok((false, false));
        }
        let result = (|| {
            let automation: IUIAutomation =
                match unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) } {
                    Ok(automation) => automation,
                    Err(_) => return Ok((false, false)),
                };
            let (mut message, mut choices) =
                match inspect_with_uia(&automation, record.dialog_hwnd, &record.public.title) {
                    Ok(snapshot) => snapshot,
                    Err(_) => return Ok((false, false)),
                };
            if message.trim().is_empty() || choices.is_empty() {
                let (win32_message, win32_choices) =
                    inspect_win32_children(record.dialog_hwnd, &record.public.title);
                if message.trim().is_empty() {
                    message = win32_message;
                }
                if choices.is_empty() {
                    choices = win32_choices;
                }
            }
            for (index, candidate) in choices.iter_mut().enumerate() {
                candidate.public.id = format!("choice-{index}");
                candidate.ordinal = index;
            }
            let fingerprint = dialog_fingerprint(
                &record.public.title,
                &message,
                &window_class(record.dialog_hwnd),
                &choices,
            );
            if fingerprint != record.fingerprint {
                return Err(
                    "Unity dialog content changed before the choice was invoked".to_string()
                );
            }

            let root = unsafe { automation.ElementFromHandle(hwnd(record.dialog_hwnd)) }
                .map_err(|error| error.to_string())?;
            let condition =
                unsafe { automation.CreateTrueCondition() }.map_err(|error| error.to_string())?;
            let elements = unsafe { root.FindAll(TreeScope_Descendants, &condition) }
                .map_err(|error| error.to_string())?;
            let length = unsafe { elements.Length() }
                .map_err(|error| error.to_string())?
                .clamp(0, MAX_UIA_ELEMENTS);
            let mut button_ordinal = 0usize;
            for index in 0..length {
                let element =
                    unsafe { elements.GetElement(index) }.map_err(|error| error.to_string())?;
                let control_type =
                    unsafe { element.CurrentControlType() }.map_err(|error| error.to_string())?;
                let class_name = unsafe { element.CurrentClassName() }
                    .map_err(|error| error.to_string())?
                    .to_string();
                if control_type != UIA_ButtonControlTypeId
                    && !class_name.eq_ignore_ascii_case("Button")
                {
                    continue;
                }
                let name = unsafe { element.CurrentName() }
                    .map_err(|error| error.to_string())?
                    .to_string();
                let matches =
                    button_ordinal == choice.ordinal && name.trim() == choice.public.label;
                button_ordinal += 1;
                if !matches {
                    continue;
                }
                let Ok(pattern) = (unsafe {
                    element.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId)
                }) else {
                    return Ok((false, true));
                };
                if unsafe { pattern.Invoke() }.is_err() {
                    return Ok((false, true));
                }
                return Ok((true, true));
            }
            Ok((false, true))
        })();
        unsafe { CoUninitialize() };
        result
    }

    #[derive(Default)]
    struct TopLevelContext {
        process_id: u32,
        main_hwnd: isize,
        windows: Vec<isize>,
    }

    unsafe extern "system" fn enum_owned_windows_callback(window: HWND, parameter: LPARAM) -> BOOL {
        let context = &mut *(parameter.0 as *mut TopLevelContext);
        let value = window.0 as isize;
        if value != context.main_hwnd
            && window_matches_process(value, context.process_id)
            && window_visible(value)
            && owner_chain_reaches(value, context.main_hwnd)
        {
            context.windows.push(value);
        }
        BOOL(1)
    }

    fn owned_top_level_windows(process_id: u32, main_hwnd: isize) -> Vec<isize> {
        let mut context = TopLevelContext {
            process_id,
            main_hwnd,
            windows: Vec::new(),
        };
        let _ = unsafe {
            EnumWindows(
                Some(enum_owned_windows_callback),
                LPARAM((&mut context as *mut TopLevelContext) as isize),
            )
        };
        context.windows
    }

    #[derive(Default)]
    struct MainWindowContext {
        process_id: u32,
        candidates: Vec<(i64, isize)>,
    }

    unsafe extern "system" fn enum_main_windows_callback(window: HWND, parameter: LPARAM) -> BOOL {
        let context = &mut *(parameter.0 as *mut MainWindowContext);
        let value = window.0 as isize;
        if !window_matches_process(value, context.process_id) || !window_visible(value) {
            return BOOL(1);
        }
        if GetWindow(window, GW_OWNER)
            .ok()
            .map(|owner| !owner.0.is_null())
            .unwrap_or(false)
        {
            return BOOL(1);
        }
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect).is_err() {
            return BOOL(1);
        }
        let width = i64::from((rect.right - rect.left).max(0));
        let height = i64::from((rect.bottom - rect.top).max(0));
        let class_bonus = if window_class(value).contains("Unity") {
            1_000_000_000i64
        } else {
            0
        };
        context
            .candidates
            .push((class_bonus + width * height, value));
        BOOL(1)
    }

    fn find_unity_main_window(process_id: u32) -> Option<isize> {
        let mut context = MainWindowContext {
            process_id,
            candidates: Vec::new(),
        };
        let _ = unsafe {
            EnumWindows(
                Some(enum_main_windows_callback),
                LPARAM((&mut context as *mut MainWindowContext) as isize),
            )
        };
        context
            .candidates
            .into_iter()
            .max_by_key(|(score, _)| *score)
            .map(|(_, hwnd)| hwnd)
    }

    fn hwnd(value: isize) -> HWND {
        HWND(value as *mut c_void)
    }

    fn root_window(value: isize) -> isize {
        if value == 0 {
            return 0;
        }
        unsafe { GetAncestor(hwnd(value), GA_ROOT).0 as isize }
    }

    fn owner_root(value: isize) -> isize {
        root_window(value)
    }

    fn owner_chain_reaches(mut value: isize, target: isize) -> bool {
        for _ in 0..16 {
            let Ok(owner) = (unsafe { GetWindow(hwnd(value), GW_OWNER) }) else {
                return false;
            };
            let owner = owner.0 as isize;
            if owner == 0 {
                return false;
            }
            if owner == target {
                return true;
            }
            value = owner;
        }
        false
    }

    fn window_matches_process(value: isize, process_id: u32) -> bool {
        if value == 0 || !unsafe { IsWindow(Some(hwnd(value))) }.as_bool() {
            return false;
        }
        let mut actual = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd(value), Some(&mut actual)) };
        actual == process_id
    }

    fn window_visible(value: isize) -> bool {
        value != 0 && unsafe { IsWindowVisible(hwnd(value)) }.as_bool()
    }

    fn window_enabled(value: isize) -> bool {
        value != 0 && unsafe { IsWindowEnabled(hwnd(value)) }.as_bool()
    }

    fn window_text(value: isize) -> String {
        if value == 0 {
            return String::new();
        }
        let length = unsafe { GetWindowTextLengthW(hwnd(value)) }.max(0) as usize;
        let mut buffer = vec![0u16; length.saturating_add(1).max(2)];
        let copied = unsafe { GetWindowTextW(hwnd(value), &mut buffer) }.max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied])
            .trim()
            .to_string()
    }

    fn control_text(value: isize) -> String {
        if value == 0 {
            return String::new();
        }
        let target = hwnd(value);
        let length =
            unsafe { SendMessageW(target, WM_GETTEXTLENGTH, None, None).0 }.max(0) as usize;
        let mut buffer = vec![0u16; length.saturating_add(1).max(2)];
        let copied = unsafe {
            SendMessageW(
                target,
                WM_GETTEXT,
                Some(WPARAM(buffer.len())),
                Some(LPARAM(buffer.as_mut_ptr() as isize)),
            )
            .0
        }
        .max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied.min(buffer.len())])
            .trim()
            .to_string()
    }

    fn window_class(value: isize) -> String {
        if value == 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; 256];
        let copied = unsafe { GetClassNameW(hwnd(value), &mut buffer) }.max(0) as usize;
        String::from_utf16_lossy(&buffer[..copied])
            .trim()
            .to_string()
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn blocked_error_contains_dialog_and_on_demand_sdk_usage() {
            let dialog = UnityModalDialog {
                code: UNITY_MODAL_DIALOG_BLOCKED_CODE.to_string(),
                dialog_id: "dialog-test".to_string(),
                project: r"F:\Project".to_string(),
                title: "Scene changed".to_string(),
                message: "Reload it?".to_string(),
                choices: vec![UnityDialogChoice {
                    id: "choice-0".to_string(),
                    label: "Reload".to_string(),
                }],
                main_thread_blocked: true,
                opened_at_ms: 1,
            };
            let error = format_blocked_error(&dialog, "not_sent", None);
            assert!(error.contains("code=unity_modal_dialog_blocked"));
            assert!(error.contains("Unity 主线程已被模态弹窗阻塞"));
            assert!(error.contains("choice-0: Reload"));
            assert!(error.contains("locus.choose_unity_dialog"));
            assert!(error.contains("可安全重试"));

            let detached = format_blocked_error(&dialog, "detached", Some("exec-test"));
            assert!(detached.contains("locus.wait_unity_execution"));
            assert!(detached.contains("exec-test"));
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    use tokio::sync::watch;

    pub async fn ensure_project_observed(_project_path: &str) -> Result<(), String> {
        Ok(())
    }

    pub async fn sync_project_process(
        _project_path: &str,
        _process_id: Option<u32>,
        _process_created_at_ms: Option<u64>,
    ) {
    }

    pub fn subscribe() -> watch::Receiver<u64> {
        watch::channel(0).1
    }

    pub fn current_dialog(_project_path: &str) -> Option<UnityModalDialog> {
        None
    }

    pub fn blocked_error(
        _project_path: &str,
        _request_state: &str,
        _request_id: Option<&str>,
    ) -> Option<String> {
        None
    }

    pub async fn choose_dialog(
        _project_path: &str,
        _dialog_id: &str,
        _choice_id: &str,
    ) -> Result<UnityDialogChoiceResult, String> {
        Err("Unity modal dialog recovery is currently supported on Windows".to_string())
    }
}

pub use platform::{
    blocked_error, choose_dialog, current_dialog, ensure_project_observed, subscribe,
    sync_project_process,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_thread_message_classification_keeps_recovery_channels_available() {
        for message in [
            "ping",
            "status",
            "bridge_capabilities",
            "cancel_execute_code",
            "execute_code_progress",
            "get_reload_state",
            "get_compile_result",
        ] {
            assert!(!message_requires_unity_main_thread(message), "{message}");
        }
        for message in [
            "execute_code",
            "execute_loaded",
            "execute_code_wait",
            "request_recompile",
            "set_editor_status",
            "run_states",
            "capture_viewport",
            "read_yaml",
            "property_tree_write",
        ] {
            assert!(message_requires_unity_main_thread(message), "{message}");
        }
    }
}
