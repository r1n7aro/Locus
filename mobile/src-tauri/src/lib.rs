//! Locus Remote: the phone side of Locus mobile remote access.
//!
//! The bundled launcher pairs with desktops (QR code with certificate pin,
//! or a 6-digit code over CA-validated HTTPS) and the app then shows the page
//! each desktop serves, relaying it and every RPC over the pinned connection.

mod asset_cache;
pub mod client;
mod commands;
pub mod connection;
mod proxy;
mod secure;
mod store;
pub mod tls;

use tauri::Manager;

use commands::AppState;
use secure::Sealer;
use store::DesktopStore;

#[cfg(target_os = "android")]
struct SealerSlot(Sealer);

/// Registers `SecureStorePlugin.kt` (AndroidKeyStore). Its commands are not
/// granted to any page; only Rust calls it.
#[cfg(target_os = "android")]
fn secure_store_plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri::plugin::Builder::new("secure-store")
        .setup(|app, api| {
            let handle = api.register_android_plugin("com.locus.remote", "SecureStorePlugin")?;
            app.manage(SealerSlot(Sealer::Keystore(handle)));
            Ok(())
        })
        .build()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .register_asynchronous_uri_scheme_protocol(proxy::SCHEME, |context, request, responder| {
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(proxy::handle(&app, request).await);
            });
        })
        .register_asynchronous_uri_scheme_protocol(proxy::BINARY_SCHEME, |context, request, responder| {
            let app = context.app_handle().clone();
            tauri::async_runtime::spawn(async move {
                responder.respond(proxy::handle_binary(&app, request).await);
            });
        });
    #[cfg(mobile)]
    let builder = builder.plugin(tauri_plugin_barcode_scanner::init());
    #[cfg(target_os = "android")]
    let builder = builder.plugin(secure_store_plugin());

    builder
        .setup(|app| {
            #[allow(unused_mut)]
            let mut data_dir = app.path().app_data_dir()?;
            #[allow(unused_mut)]
            let mut cache_dir = app.path().app_cache_dir()?;
            // Debug builds only: keeps test runs out of the user's profile.
            #[cfg(debug_assertions)]
            if let Some(dir) = std::env::var_os("LOCUS_REMOTE_DATA_DIR") {
                data_dir = std::path::PathBuf::from(dir);
                cache_dir = data_dir.join("cache");
            }
            #[cfg(target_os = "android")]
            let sealer = app.state::<SealerSlot>().0.clone();
            #[cfg(not(target_os = "android"))]
            let sealer = Sealer::Plain;
            let cache = asset_cache::AssetCache::new(cache_dir.join("page-assets"));
            app.manage(AppState::new(DesktopStore::load(&data_dir), sealer, cache));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::launcher_list_desktops,
            commands::launcher_pair_with_link,
            commands::launcher_pair_with_code,
            commands::launcher_connect,
            commands::launcher_forget,
            commands::remote_subscribe,
            commands::remote_invoke,
            commands::remote_status,
            commands::remote_wake,
            commands::remote_leave,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Locus Remote");
}
