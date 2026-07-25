use std::ffi::OsString;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use windows_service::{
    define_windows_service,
    service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    },
    service_control_handler::{self, ServiceControlHandlerResult, ServiceStatusHandle},
    service_dispatcher,
};

#[path = "../../src/windows_job.rs"]
mod windows_job;

const SERVICE_NAME: &str = "MavroDPI";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CHILD_RESTART_DELAY: Duration = Duration::from_secs(3);

struct ManagedChild {
    child: Child,
    _job: windows_job::KillOnCloseJob,
}

impl ManagedChild {
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    fn kill(&mut self) -> std::io::Result<()> {
        self.child.kill()
    }

    fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        self.child.wait()
    }
}

define_windows_service!(ffi_service_main, service_main);

fn main() -> windows_service::Result<()> {
    service_dispatcher::start(SERVICE_NAME, ffi_service_main)
}

fn service_main(_args: Vec<OsString>) {
    let _ = run_service();
}

fn set_status(
    handle: &ServiceStatusHandle,
    state: ServiceState,
    controls: ServiceControlAccept,
    exit_code: ServiceExitCode,
    wait_hint: Duration,
) -> windows_service::Result<()> {
    handle.set_service_status(ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: controls,
        exit_code,
        checkpoint: if matches!(
            state,
            ServiceState::StartPending | ServiceState::StopPending
        ) {
            1
        } else {
            0
        },
        wait_hint,
        process_id: None,
    })
}

fn spawn_goodbyedpi(
    executable: &std::path::Path,
    working_dir: &std::path::Path,
) -> std::io::Result<ManagedChild> {
    let mut child = Command::new(executable)
        .arg("-5")
        .current_dir(working_dir)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    let job = match windows_job::KillOnCloseJob::assign(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };

    // Servisi "çalışıyor" olarak bildirmeden önce motorun ilk açılışta
    // (ör. eksik sürücü nedeniyle) hemen kapanmadığını doğrula.
    std::thread::sleep(Duration::from_millis(750));
    match child.try_wait() {
        Ok(None) => Ok(ManagedChild { child, _job: job }),
        Ok(Some(status)) => Err(std::io::Error::other(format!(
            "GoodbyeDPI hemen kapandı (çıkış durumu: {status})."
        ))),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    }
}

fn stopped_exit_code(error: &std::io::Error) -> ServiceExitCode {
    ServiceExitCode::Win32(error.raw_os_error().unwrap_or(1) as u32)
}

fn run_service() -> windows_service::Result<()> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_signal = Arc::clone(&stop);

    let handle = service_control_handler::register(SERVICE_NAME, move |control| match control {
        ServiceControl::Stop => {
            stop_signal.store(true, Ordering::SeqCst);
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;

    set_status(
        &handle,
        ServiceState::StartPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::Win32(0),
        Duration::from_secs(10),
    )?;

    let current_exe = std::env::current_exe().map_err(windows_service::Error::Winapi)?;
    let dir = current_exe
        .parent()
        .ok_or_else(|| {
            windows_service::Error::Winapi(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Servis çalışma dizini bulunamadı.",
            ))
        })?
        .to_path_buf();
    let executable = dir.join("goodbyedpi.exe");

    let mut child = match spawn_goodbyedpi(&executable, &dir) {
        Ok(child) => child,
        Err(error) => {
            let _ = set_status(
                &handle,
                ServiceState::Stopped,
                ServiceControlAccept::empty(),
                stopped_exit_code(&error),
                Duration::default(),
            );
            return Err(windows_service::Error::Winapi(error));
        }
    };

    set_status(
        &handle,
        ServiceState::Running,
        ServiceControlAccept::STOP,
        ServiceExitCode::Win32(0),
        Duration::default(),
    )?;

    while !stop.load(Ordering::SeqCst) {
        let child_stopped = match child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                true
            }
        };

        if child_stopped {
            thread_sleep_interruptible(&stop, CHILD_RESTART_DELAY);
            if stop.load(Ordering::SeqCst) {
                break;
            }

            child = match spawn_goodbyedpi(&executable, &dir) {
                Ok(child) => child,
                Err(error) => {
                    let _ = set_status(
                        &handle,
                        ServiceState::Stopped,
                        ServiceControlAccept::empty(),
                        stopped_exit_code(&error),
                        Duration::default(),
                    );
                    return Err(windows_service::Error::Winapi(error));
                }
            };
        }

        thread_sleep_interruptible(&stop, Duration::from_millis(250));
    }

    set_status(
        &handle,
        ServiceState::StopPending,
        ServiceControlAccept::empty(),
        ServiceExitCode::Win32(0),
        Duration::from_secs(5),
    )?;

    let _ = child.kill();
    let _ = child.wait();

    set_status(
        &handle,
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        ServiceExitCode::Win32(0),
        Duration::default(),
    )
}

fn thread_sleep_interruptible(stop: &AtomicBool, duration: Duration) {
    let interval = Duration::from_millis(100);
    let mut remaining = duration;
    while !stop.load(Ordering::SeqCst) && remaining > Duration::ZERO {
        let sleep_for = remaining.min(interval);
        std::thread::sleep(sleep_for);
        remaining = remaining.saturating_sub(sleep_for);
    }
}
