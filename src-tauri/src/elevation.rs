// Windows yönetici (UAC) yetkisi yardımcıları.
// WinDivert çekirdek sürücüsü yüklediği için GoodbyeDPI admin gerektirir.

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut core::ffi::c_void,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

// Uygulamayı yönetici olarak yeniden başlatır (UAC istemi gösterir).
// Başarılıysa true döner; çağıran taraf mevcut süreçten çıkmalıdır.
#[cfg(windows)]
pub fn relaunch_as_admin() -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let exe_w: Vec<u16> = exe
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let verb: Vec<u16> = "runas\0".encode_utf16().collect();
    let parameters: Vec<u16> = "--elevated-relaunch\0".encode_utf16().collect();

    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            exe_w.as_ptr(),
            parameters.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW > 32 ise başarılı.
    (result as isize) > 32
}

#[cfg(windows)]
pub struct SingleInstanceGuard(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::ReleaseMutex;

        unsafe {
            let _ = ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
pub fn acquire_single_instance(wait_for_previous_ms: u32) -> Option<SingleInstanceGuard> {
    use windows_sys::Win32::Foundation::{
        GetLastError, ERROR_ALREADY_EXISTS, WAIT_ABANDONED, WAIT_OBJECT_0,
    };
    use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};

    // DNS durumu makine genelinde olduğu için masaüstü örneklerini Windows
    // oturumları arasında da tekilleştir.
    let mutex_name: Vec<u16> = "Global\\MavroDPI-Desktop-v1\0".encode_utf16().collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, mutex_name.as_ptr()) };
    if handle.is_null() {
        return None;
    }

    let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if !already_exists {
        return Some(SingleInstanceGuard(handle));
    }

    let wait_result = unsafe { WaitForSingleObject(handle, wait_for_previous_ms) };
    if wait_result == WAIT_OBJECT_0 || wait_result == WAIT_ABANDONED {
        Some(SingleInstanceGuard(handle))
    } else {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(handle);
        }
        None
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    true
}

#[cfg(not(windows))]
pub fn relaunch_as_admin() -> bool {
    false
}
