mod checker;
mod dns;
mod elevation;
mod engine;
mod service;
mod stats;
#[cfg(windows)]
mod windows_acl;
#[cfg(windows)]
mod windows_job;

use engine::EngineState;
use stats::NetState;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::Manager;

static EXIT_CLEANUP_STARTED: AtomicBool = AtomicBool::new(false);

fn cleanup_owned_runtime(app: &tauri::AppHandle) {
    if EXIT_CLEANUP_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    let state = app.state::<EngineState>();
    engine::stop_internal(state.inner());
    let _ = dns::disable_doh_internal(app);
}

#[tauri::command]
fn is_admin() -> bool {
    elevation::is_elevated()
}

#[tauri::command]
fn relaunch_admin(app: tauri::AppHandle) -> bool {
    if elevation::relaunch_as_admin() {
        app.exit(0);
        true
    } else {
        false
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
                cleanup_owned_runtime(app);
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
    let repair_service_after_update = std::env::args_os()
        .any(|argument| argument == std::ffi::OsStr::new("--repair-service-after-update"));

    if std::env::args_os()
        .any(|argument| argument == std::ffi::OsStr::new("--cleanup-before-uninstall"))
    {
        let mut cleanup_errors = Vec::new();
        if let Err(error) = dns::restore_persisted_snapshot() {
            cleanup_errors.push(format!("Ağ ayarları geri yüklenemedi: {error}"));
        }
        if let Err(error) = service::uninstall_service() {
            cleanup_errors.push(format!("Windows servisi kaldırılamadı: {error}"));
        }

        if cleanup_errors.is_empty() {
            std::process::exit(0);
        }

        eprintln!("{}", cleanup_errors.join("\n"));
        std::process::exit(1);
    }

    #[cfg(windows)]
    let _single_instance_guard = {
        let elevated_relaunch = std::env::args_os()
            .any(|argument| argument == std::ffi::OsStr::new("--elevated-relaunch"));
        let wait_ms = if elevated_relaunch { 15_000 } else { 5_000 };
        match elevation::acquire_single_instance(wait_ms) {
            Some(guard) => guard,
            None => return,
        }
    };

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(EngineState::default())
        .manage(NetState::default())
        .invoke_handler(tauri::generate_handler![
            engine::start_protection,
            engine::stop_protection,
            engine::get_status,
            engine::get_engine_info,
            stats::net_stats,
            dns::enable_doh,
            dns::disable_doh,
            dns::doh_managed,
            dns::reset_unmanaged_dns,
            dns::doh_status,
            service::service_installed,
            service::service_status,
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
        .setup(move |app| {
            if repair_service_after_update {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
                return Ok(());
            }

            // Önceki oturum beklenmedik biçimde kapandıysa DNS ayarlarını
            // uygulama başlamadan önce kayıtlı özgün haline geri döndür.
            dns::recover_stale_snapshot(app.handle());
            engine::start_monitor(app.handle().clone());
            build_tray(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    if repair_service_after_update {
        let repair_result = service::service_status().and_then(|status| {
            if status.installed && status.needs_repair {
                service::install_service(app.handle().clone())
            } else {
                Ok(())
            }
        });
        if let Err(error) = repair_result {
            eprintln!("Windows servisi güncelleme sonrasında onarılamadı: {error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    app.run(|app, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            cleanup_owned_runtime(app);
        }
    });
}
