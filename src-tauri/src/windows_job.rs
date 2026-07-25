#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use std::process::Child;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// Keeps a child process in a kill-on-close Windows job. If the owning
/// application/service crashes or is terminated, Windows closes this handle
/// and terminates the child before it can leave packet filtering behind.
#[cfg(windows)]
pub struct KillOnCloseJob(HANDLE);

// SAFETY: A Windows kernel handle is process-wide, not thread-affine. This
// wrapper only closes it on drop and never dereferences the raw pointer value.
#[cfg(windows)]
unsafe impl Send for KillOnCloseJob {}

#[cfg(windows)]
impl KillOnCloseJob {
    pub fn assign(child: &Child) -> std::io::Result<Self> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }

            let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &information as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }

            if AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) == 0 {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }

            Ok(Self(job))
        }
    }
}

#[cfg(windows)]
impl Drop for KillOnCloseJob {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
