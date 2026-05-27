use std::ffi::OsString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};

const SERVICE_NAME: &str = "MavroDPI";

define_windows_service!(ffi_service_main, service_main);

fn main() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_args: Vec<OsString>) {
    let _ = run_service();
}

fn run_service() -> windows_service::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();

    let handle = service_control_handler::register(SERVICE_NAME, move |ctrl| match ctrl {
        ServiceControl::Stop => {
            stop2.store(true, Ordering::SeqCst);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;

    handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Running,
        controls_accepted: ServiceControlAccept::STOP,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    let dir = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let exe = dir.join("goodbyedpi.exe");
    let mut child: Option<std::process::Child> = None;

    while !stop.load(Ordering::SeqCst) {
        let dead = child.as_mut().map_or(true, |c| {
            c.try_wait().map_or(false, |s| s.is_some())
        });
        if dead {
            child = std::process::Command::new(&exe)
                .arg("-5")
                .current_dir(&dir)
                .spawn()
                .ok();
        }
        std::thread::sleep(Duration::from_secs(3));
    }

    if let Some(mut c) = child {
        let _ = c.kill();
        let _ = c.wait();
    }

    handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: ServiceState::Stopped,
        controls_accepted: ServiceControlAccept::empty(),
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::default(),
        process_id: None,
    })?;

    Ok(())
}
