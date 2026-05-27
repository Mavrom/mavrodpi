mod checker;
mod dns;
mod elevation;
mod engine;
mod service;
mod stats;

use engine::EngineState;
use stats::NetState;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

#[tauri::command]
fn is_admin() -> bool {
    elevation::is_elevated()
}

#[tauri::command]
fn relaunch_admin(app: tauri::AppHandle) {
    if elevation::relaunch_as_admin() {
        app.exit(0);
    }
}

fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "Göster", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Çıkış", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let _tray = TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("MavroDPI")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => {
                let state = app.state::<EngineState>();
                engine::stop_internal(state.inner());
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
        })
        .build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Release modunda WinDivert için yönetici yetkisi şart: değilsek
    // UAC ile kendimizi yeniden başlatıp mevcut süreçten çıkarız.
    #[cfg(all(windows, not(debug_assertions)))]
    {
        if !elevation::is_elevated() && elevation::relaunch_as_admin() {
            std::process::exit(0);
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .manage(EngineState::default())
        .manage(NetState::default())
        .invoke_handler(tauri::generate_handler![
            engine::start_protection,
            engine::stop_protection,
            engine::get_status,
            stats::net_stats,
            dns::enable_doh,
            dns::disable_doh,
            dns::doh_status,
            service::service_installed,
            service::install_service,
            service::uninstall_service,
            service::start_service_now,
            checker::check_sites,
            is_admin,
            relaunch_admin
        ])
        .on_window_event(|window, event| {
            // Pencere kapatılınca uygulamayı kapatma — tepsiye küçült.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .setup(|app| {
            build_tray(app)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
