//! The pairing QR code's landing page and the Locus Remote APK bundled with
//! the `with_remote_app` installer flavor.
//!
//! The QR code always points at `/app`, so scanning it with a phone camera
//! (instead of the app) opens this page rather than a certificate warning for
//! the pinned self-signed certificate. With a bundled APK the page offers it
//! for a while after a pairing window opens, so a phone without the app can
//! install it from the LAN; otherwise it explains that the code belongs in
//! the app and links the official release page. The page and the APK are the
//! only content the NIC listeners serve over plain HTTP (see
//! `server::run_listener`).
//!
//! Layout inside the resource directory (written by
//! `scripts/prepare-remote-app.mjs`):
//!   remote-app/locus-remote.apk
//!   remote-app/app.json   {"version", "sha256", "size"}

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use hyper::body::Bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager};

/// How long after a pairing window opens the download stays available:
/// enough to download, allow the install, install and scan.
pub const DOWNLOAD_GRACE: Duration = Duration::from_secs(10 * 60);
pub const PAGE_PATH: &str = "/app";
pub const APK_PATH: &str = "/app/locus-remote.apk";
const RESOURCE_DIR: &str = "remote-app";
const APK_FILE: &str = "locus-remote.apk";
const METADATA_FILE: &str = "app.json";
/// Refuses anything implausible as an APK before reading it into memory.
const MAX_APK_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
struct Metadata {
    version: String,
    sha256: String,
    size: u64,
}

#[derive(Debug, Clone)]
pub struct AppPackage {
    path: PathBuf,
    pub version: String,
    pub size: u64,
    sha256: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppPackageView {
    pub version: String,
    pub size_bytes: u64,
}

impl AppPackage {
    pub fn view(&self) -> AppPackageView {
        AppPackageView { version: self.version.clone(), size_bytes: self.size }
    }

    pub fn file_name(&self) -> String {
        let version: String = self
            .version
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
            .take(32)
            .collect();
        if version.is_empty() {
            "LocusRemote.apk".into()
        } else {
            format!("LocusRemote-{version}.apk")
        }
    }

    /// Reads the APK and checks it against the recorded digest, so a damaged
    /// or swapped file in the install directory is never offered.
    pub async fn read(&self) -> Result<Bytes, String> {
        let path = self.path.clone();
        let expected = self.sha256.clone();
        let size = self.size;
        tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
            if bytes.len() as u64 != size || hex_sha256(&bytes) != expected {
                return Err(format!("{} does not match its recorded digest", path.display()));
            }
            Ok(Bytes::from(bytes))
        })
        .await
        .map_err(|error| error.to_string())?
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn from_dir(dir: &Path) -> Option<AppPackage> {
    let apk = dir.join(APK_FILE);
    let raw = std::fs::read(dir.join(METADATA_FILE)).ok()?;
    let metadata: Metadata = match serde_json::from_slice(&raw) {
        Ok(metadata) => metadata,
        Err(error) => {
            eprintln!("[RemoteAccess] ignoring {}: {error}", dir.join(METADATA_FILE).display());
            return None;
        }
    };
    let size = std::fs::metadata(&apk).ok()?.len();
    let digest_ok = metadata.sha256.len() == 64 && metadata.sha256.bytes().all(|byte| byte.is_ascii_hexdigit());
    if size != metadata.size || size > MAX_APK_BYTES || !digest_ok {
        eprintln!("[RemoteAccess] ignoring bundled app: size or digest record is invalid");
        return None;
    }
    Some(AppPackage {
        path: apk,
        version: metadata.version,
        size,
        sha256: metadata.sha256.to_ascii_lowercase(),
    })
}

/// Development builds: `LOCUS_REMOTE_APK=<path to an apk>` stands in for the
/// bundled package; its digest is taken as is.
#[cfg(debug_assertions)]
fn from_env() -> Option<AppPackage> {
    let path = PathBuf::from(std::env::var_os("LOCUS_REMOTE_APK")?);
    let bytes = std::fs::read(&path).ok()?;
    Some(AppPackage {
        size: bytes.len() as u64,
        sha256: hex_sha256(&bytes),
        version: "dev".into(),
        path,
    })
}

#[cfg(not(debug_assertions))]
fn from_env() -> Option<AppPackage> {
    None
}

/// The bundled package, looked up once per process.
pub fn bundled(app: &AppHandle) -> Option<&'static AppPackage> {
    static PACKAGE: OnceLock<Option<AppPackage>> = OnceLock::new();
    PACKAGE
        .get_or_init(|| {
            from_env().or_else(|| {
                let dir = app.path().resource_dir().ok()?.join(RESOURCE_DIR);
                from_dir(&dir)
            })
        })
        .as_ref()
}

/// When the download page offers the APK.
#[derive(Default)]
pub struct DownloadWindow {
    open_until: std::sync::Mutex<Option<Instant>>,
    cache: tokio::sync::Mutex<Option<Bytes>>,
}

impl DownloadWindow {
    pub fn open(&self, now: Instant) {
        *self.open_until.lock().unwrap_or_else(|error| error.into_inner()) = Some(now + DOWNLOAD_GRACE);
    }

    pub fn is_open(&self, now: Instant) -> bool {
        self.open_until
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some_and(|until| until > now)
    }

    /// Closes the window and frees the cached APK.
    pub fn close(&self) {
        *self.open_until.lock().unwrap_or_else(|error| error.into_inner()) = None;
        if let Ok(mut cache) = self.cache.try_lock() {
            cache.take();
        }
    }

    /// The APK bytes, read once and shared by concurrent downloads.
    pub async fn bytes(&self, package: &AppPackage) -> Result<Bytes, String> {
        let mut cache = self.cache.lock().await;
        if let Some(bytes) = cache.as_ref() {
            return Ok(bytes.clone());
        }
        let bytes = package.read().await?;
        *cache = Some(bytes.clone());
        Ok(bytes)
    }
}

// ----- download page ------------------------------------------------------------

/// Removes the pairing payload from the address bar and history: it is a
/// one-time secret meant for the app, not for the browser.
const PAGE_SCRIPT: &str = "history.replaceState(null,'',location.pathname)";

pub fn page_csp() -> &'static str {
    static CSP: OnceLock<String> = OnceLock::new();
    CSP.get_or_init(|| {
        use base64::Engine;
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(PAGE_SCRIPT.as_bytes()));
        format!(
            "default-src 'none'; style-src 'unsafe-inline'; script-src 'sha256-{hash}'; \
             img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'"
        )
    })
}

/// Where the official builds (including the standalone APK) are published.
const RELEASES_URL: &str = "https://github.com/r1n7aro/Locus/releases/latest";

#[derive(Debug, Clone, Copy)]
pub enum PageState<'a> {
    /// The bundled APK can be downloaded right now.
    Available(&'a AppPackage),
    /// The download is only offered shortly after a pairing window opened.
    Closed,
    /// This desktop offers no APK (none bundled, or the download is turned
    /// off): explain that the code is meant for the app.
    NotOffered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Client {
    Android,
    Ios,
    WeChat,
    Other,
}

fn client(user_agent: &str) -> Client {
    if user_agent.contains("MicroMessenger") {
        Client::WeChat
    } else if user_agent.contains("iPhone") || user_agent.contains("iPad") {
        Client::Ios
    } else if user_agent.contains("Android") {
        Client::Android
    } else {
        Client::Other
    }
}

fn prefers_chinese(accept_language: &str) -> bool {
    accept_language
        .split(',')
        .next()
        .is_some_and(|first| first.trim().to_ascii_lowercase().starts_with("zh"))
        || accept_language.is_empty()
}

fn escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn format_size(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

struct Text {
    lang: &'static str,
    tagline: &'static str,
    download: &'static str,
    steps: [&'static str; 3],
    trust: &'static str,
    closed_title: &'static str,
    closed_body: &'static str,
    app_only_title: &'static str,
    app_only_body: &'static str,
    app_only_steps: [&'static str; 3],
    get_app: &'static str,
    releases: &'static str,
    ios: &'static str,
    wechat: &'static str,
    other: &'static str,
}

const ZH: Text = Text {
    lang: "zh-CN",
    tagline: "在手机上查看和继续电脑上的 Locus 会话",
    download: "下载 Android 版",
    steps: [
        "下载并安装。如果系统提示「未知来源」，允许浏览器安装应用。",
        "打开 Locus Remote，点「扫码配对」。",
        "再扫一次电脑上的二维码；二维码过期了就在电脑上点「重新生成二维码」。",
    ],
    trust: "安装包由你电脑上的 Locus 直接提供。请只在信任当前网络时安装，例如家里的 Wi-Fi。",
    closed_title: "下载入口已关闭",
    closed_body: "在电脑上打开 Locus → 设置 → 远程访问，点「配对新设备」，再用相机扫描新的二维码。",
    app_only_title: "请在 Locus Remote 里扫码",
    app_only_body: "这是 Locus 的配对二维码，需要用 Locus Remote App 扫描，在浏览器里打开没有作用。",
    app_only_steps: [
        "在手机上打开 Locus Remote，点「扫码配对」。",
        "扫描电脑上的二维码；二维码过期了就在电脑上点「重新生成二维码」。",
        "在电脑上确认配对请求。",
    ],
    get_app: "还没有 App？从官方发布页下载 Android 安装包（文件名以 locus-remote_ 开头、以 _android-arm64.apk 结尾）。",
    releases: "打开发布页",
    ios: "Locus Remote 暂时只有 Android 版。",
    wechat: "微信里无法下载安装包：点右上角的「···」，选择「在浏览器中打开」。",
    other: "请用 Android 手机打开这个页面。",
};

const EN: Text = Text {
    lang: "en",
    tagline: "Browse and continue your desktop Locus sessions on your phone",
    download: "Download for Android",
    steps: [
        "Download and install it. If Android asks about unknown sources, allow your browser to install apps.",
        "Open Locus Remote and tap \u{201c}Scan to pair\u{201d}.",
        "Scan the QR code on the desktop again. If it expired, regenerate it on the desktop.",
    ],
    trust: "This package comes straight from Locus on your computer. Only install it on a network you trust, such as your home Wi-Fi.",
    closed_title: "Download closed",
    closed_body: "On the desktop, open Locus \u{2192} Settings \u{2192} Remote access, click \u{201c}Pair new device\u{201d} and scan the new QR code with your camera.",
    app_only_title: "Scan this code in Locus Remote",
    app_only_body: "This is a Locus pairing code. Scan it with the Locus Remote app; opening it in a browser does nothing.",
    app_only_steps: [
        "Open Locus Remote on your phone and tap \u{201c}Scan to pair\u{201d}.",
        "Scan the QR code on the desktop. If it expired, regenerate it on the desktop.",
        "Approve the request on the desktop.",
    ],
    get_app: "No app yet? Download the Android package from the official release page (locus-remote_\u{2026}_android-arm64.apk).",
    releases: "Open the release page",
    ios: "Locus Remote is only available for Android for now.",
    wechat: "WeChat cannot download app packages: tap \u{201c}\u{b7}\u{b7}\u{b7}\u{201d} at the top right and open this page in your browser.",
    other: "Open this page on an Android phone.",
};

const STYLE: &str = "\
:root{color-scheme:light dark;--fg:#1d1d1f;--muted:#6e6e73;--bg:#f5f5f7;--card:#fff;--line:#e3e3e8;--accent:#4f6bed;--warn:#8a5a00;--warn-bg:#fff4dc}\
@media (prefers-color-scheme:dark){:root{--fg:#ececf1;--muted:#a1a1aa;--bg:#17171a;--card:#212126;--line:#323238;--accent:#7b8cff;--warn:#f5c26b;--warn-bg:#3a2e14}}\
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.55 system-ui,-apple-system,\"Segoe UI\",\"PingFang SC\",\"Microsoft YaHei\",sans-serif}\
main{max-width:460px;margin:0 auto;padding:max(28px,env(safe-area-inset-top)) 20px 32px}\
.brand{display:flex;align-items:center;gap:12px;margin-bottom:20px}\
.logo{display:flex;width:44px;height:44px;align-items:center;justify-content:center;border-radius:12px;background:var(--accent);color:#fff}\
h1{margin:0;font-size:20px}.tagline{margin:2px 0 0;color:var(--muted);font-size:13px}\
.card{padding:18px;border:1px solid var(--line);border-radius:14px;background:var(--card)}\
.download{display:block;padding:13px 16px;border-radius:10px;background:var(--accent);color:#fff;font-weight:600;text-align:center;text-decoration:none}\
.meta{margin:8px 0 0;color:var(--muted);font-size:12px;text-align:center}\
ol{margin:16px 0 0;padding-left:20px}li{margin:6px 0}\
.note{margin:14px 0 0;padding:10px 12px;border-radius:10px;background:var(--warn-bg);color:var(--warn);font-size:13px}\
.sub{margin:16px 0 0;padding-top:14px;border-top:1px solid var(--line);color:var(--muted);font-size:13px}\
.secondary{display:block;margin-top:10px;padding:11px 16px;border:1px solid var(--line);border-radius:10px;color:var(--accent);font-weight:600;text-align:center;text-decoration:none}\
h2{margin:0 0 6px;font-size:16px}p{margin:0}";

const PHONE_ICON: &str = "<svg width=\"22\" height=\"22\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" \
stroke-width=\"1.8\" stroke-linecap=\"round\" aria-hidden=\"true\"><rect x=\"7\" y=\"2.5\" width=\"10\" height=\"19\" rx=\"2.5\"/>\
<path d=\"M11 18.5h2\"/></svg>";

/// The page a phone camera opens from the pairing QR code.
pub fn render_page(state: PageState, user_agent: &str, accept_language: &str) -> String {
    let text = if prefers_chinese(accept_language) { &ZH } else { &EN };
    let body = match (state, client(user_agent)) {
        (_, Client::Ios) => format!("<div class=\"card\"><p>{}</p></div>", text.ios),
        (PageState::NotOffered, client) => {
            // WeChat cannot download packages from the release page either.
            let wechat = if client == Client::WeChat {
                format!("<p class=\"note\">{}</p>", text.wechat)
            } else {
                String::new()
            };
            format!(
                "<div class=\"card\"><h2>{title}</h2><p>{body}</p>\
                 <ol><li>{s1}</li><li>{s2}</li><li>{s3}</li></ol>\
                 <p class=\"sub\">{get_app}</p>\
                 <a class=\"secondary\" href=\"{RELEASES_URL}\" rel=\"noopener noreferrer\">{releases}</a>{wechat}</div>",
                title = text.app_only_title,
                body = text.app_only_body,
                s1 = text.app_only_steps[0],
                s2 = text.app_only_steps[1],
                s3 = text.app_only_steps[2],
                get_app = text.get_app,
                releases = text.releases,
            )
        }
        (_, Client::WeChat) => format!("<div class=\"card\"><p class=\"note\" style=\"margin:0\">{}</p></div>", text.wechat),
        (PageState::Closed, _) => format!(
            "<div class=\"card\"><h2>{}</h2><p>{}</p></div>",
            text.closed_title, text.closed_body
        ),
        (PageState::Available(package), client) => {
            let other = if client == Client::Other {
                format!("<p class=\"meta\">{}</p>", text.other)
            } else {
                String::new()
            };
            format!(
                "<div class=\"card\"><a class=\"download\" href=\"{APK_PATH}\" download=\"{file}\">{download}</a>\
                 <p class=\"meta\">Locus Remote {version} · {size}</p>{other}\
                 <ol><li>{s1}</li><li>{s2}</li><li>{s3}</li></ol><p class=\"note\">{trust}</p></div>",
                file = escape(&package.file_name()),
                download = text.download,
                version = escape(&package.version),
                size = format_size(package.size),
                s1 = text.steps[0],
                s2 = text.steps[1],
                s3 = text.steps[2],
                trust = text.trust,
            )
        }
    };
    format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1,viewport-fit=cover\">\
         <meta name=\"referrer\" content=\"no-referrer\"><title>Locus Remote</title>\
         <script>{PAGE_SCRIPT}</script><style>{STYLE}</style></head><body><main>\
         <div class=\"brand\"><span class=\"logo\">{PHONE_ICON}</span><div><h1>Locus Remote</h1>\
         <p class=\"tagline\">{tagline}</p></div></div>{body}</main></body></html>",
        lang = text.lang,
        tagline = text.tagline,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(dir: &Path) -> AppPackage {
        AppPackage { path: dir.join(APK_FILE), version: "0.9.2".into(), size: 5 * 1024 * 1024, sha256: "0".repeat(64) }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("locus-app-package-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bundled_package_is_checked_against_its_metadata() {
        let dir = temp_dir("metadata");
        let apk = b"PK\x03\x04 fake apk".to_vec();
        std::fs::write(dir.join(APK_FILE), &apk).unwrap();
        let metadata = |size: usize, sha: &str| format!(r#"{{"version":"1.2.3","sha256":"{sha}","size":{size}}}"#);
        std::fs::write(dir.join(METADATA_FILE), metadata(apk.len(), &hex_sha256(&apk))).unwrap();
        let package = from_dir(&dir).expect("valid package");
        assert_eq!(package.version, "1.2.3");
        assert_eq!(package.file_name(), "LocusRemote-1.2.3.apk");
        let bytes = tokio::runtime::Runtime::new().unwrap().block_on(package.read()).unwrap();
        assert_eq!(bytes.as_ref(), apk.as_slice());

        // Same size, different content: refused when read.
        std::fs::write(dir.join(APK_FILE), b"PK\x03\x04 evil apk").unwrap();
        assert!(tokio::runtime::Runtime::new().unwrap().block_on(package.read()).is_err());

        // Size mismatch: not offered at all.
        std::fs::write(dir.join(METADATA_FILE), metadata(apk.len() + 1, &hex_sha256(&apk))).unwrap();
        assert!(from_dir(&dir).is_none());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn window_opens_for_the_grace_period() {
        let window = DownloadWindow::default();
        let now = Instant::now();
        assert!(!window.is_open(now));
        window.open(now);
        assert!(window.is_open(now + DOWNLOAD_GRACE - Duration::from_secs(1)));
        assert!(!window.is_open(now + DOWNLOAD_GRACE));
        window.close();
        assert!(!window.is_open(now));
    }

    #[test]
    fn page_adapts_to_client_and_state() {
        let package = package(Path::new("."));
        let android = "Mozilla/5.0 (Linux; Android 14; Pixel 8) Chrome/130";
        let page = render_page(PageState::Available(&package), android, "zh-CN,zh;q=0.9");
        assert!(page.contains(APK_PATH) && page.contains("下载 Android 版") && page.contains("5.0 MB"));
        assert!(page.contains(PAGE_SCRIPT));
        let closed = render_page(PageState::Closed, android, "en-US");
        assert!(closed.contains("Download closed") && !closed.contains(APK_PATH));
        let wechat = render_page(PageState::Available(&package), "Android MicroMessenger/8.0", "zh-CN");
        assert!(wechat.contains("在浏览器中打开") && !wechat.contains(APK_PATH));
        let ios = render_page(PageState::Available(&package), "iPhone OS 17", "en");
        assert!(ios.contains("only available for Android") && !ios.contains(APK_PATH));
    }

    #[test]
    fn page_without_a_package_points_to_the_app() {
        let android = "Mozilla/5.0 (Linux; Android 14; Pixel 8) Chrome/130";
        let page = render_page(PageState::NotOffered, android, "zh-CN");
        assert!(page.contains("请在 Locus Remote 里扫码") && page.contains(RELEASES_URL));
        assert!(page.contains(PAGE_SCRIPT), "the pairing secret still leaves the address bar");
        assert!(!page.contains(APK_PATH) && !page.contains("在浏览器中打开"));
        let wechat = render_page(PageState::NotOffered, "Android MicroMessenger/8.0", "en-US");
        assert!(wechat.contains("Scan this code in Locus Remote") && wechat.contains("open this page in your browser"));
        let ios = render_page(PageState::NotOffered, "iPhone OS 17", "en");
        assert!(ios.contains("only available for Android") && !ios.contains(RELEASES_URL));
    }

    #[test]
    fn csp_allows_exactly_the_inline_script() {
        use base64::Engine;
        let hash = base64::engine::general_purpose::STANDARD.encode(Sha256::digest(PAGE_SCRIPT.as_bytes()));
        assert!(page_csp().contains(&format!("'sha256-{hash}'")));
        assert!(page_csp().starts_with("default-src 'none'"));
    }

    #[test]
    fn version_cannot_inject_into_the_file_name() {
        let mut package = package(Path::new("."));
        package.version = "1.0\"/../<x>".into();
        assert_eq!(package.file_name(), "LocusRemote-1.0..x.apk");
    }
}
