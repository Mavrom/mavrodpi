use std::ffi::OsStr;
use std::fs::File;
use std::io::Write;
use std::mem::{size_of, zeroed};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::Path;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, GENERIC_WRITE,
    HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SetSecurityInfo,
    SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetSecurityDescriptorControl,
    GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, ACCESS_ALLOWED_ACE, ACL,
    ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION, OBJECT_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_ATTRIBUTES, SE_DACL_PROTECTED,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateDirectoryW, CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    CREATE_NEW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, READ_CONTROL,
    WRITE_DAC, WRITE_OWNER,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;

const DIRECTORY_SDDL: &str = "O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)";
const FILE_SDDL: &str = "O:BAG:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)";

struct LocalDescriptor(PSECURITY_DESCRIPTOR);

impl LocalDescriptor {
    fn new(directory: bool) -> Result<Self, String> {
        let mut sddl = OsStr::new(if directory { DIRECTORY_SDDL } else { FILE_SDDL })
            .encode_wide()
            .collect::<Vec<_>>();
        sddl.push(0);

        let mut descriptor = null_mut();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                null_mut(),
            )
        };
        if converted == 0 || descriptor.is_null() {
            return Err(format!(
                "Windows güvenlik tanımı hazırlanamadı: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self(descriptor))
    }

    fn owner_and_dacl(&self) -> Result<(PSID, *mut ACL), String> {
        let mut owner = null_mut();
        let mut owner_defaulted = 0;
        let owner_ok =
            unsafe { GetSecurityDescriptorOwner(self.0, &mut owner, &mut owner_defaulted) };
        if owner_ok == 0 || owner.is_null() {
            return Err(format!(
                "Windows güvenlik sahibi okunamadı: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut dacl_present = 0;
        let mut dacl_defaulted = 0;
        let mut dacl = null_mut();
        let dacl_ok = unsafe {
            GetSecurityDescriptorDacl(self.0, &mut dacl_present, &mut dacl, &mut dacl_defaulted)
        };
        if dacl_ok == 0 || dacl_present == 0 || dacl.is_null() {
            return Err(format!(
                "Windows erişim listesi okunamadı: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok((owner, dacl))
    }
}

impl Drop for LocalDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
}

struct RawHandle(HANDLE);

impl RawHandle {
    fn into_file(mut self) -> File {
        let handle = self.0;
        self.0 = INVALID_HANDLE_VALUE;
        unsafe { File::from_raw_handle(handle) }
    }
}

impl Drop for RawHandle {
    fn drop(&mut self) {
        if self.0 != INVALID_HANDLE_VALUE && !self.0.is_null() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn validate_handle_type(handle: HANDLE, directory: bool, path: &Path) -> Result<(), String> {
    let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
    let result = unsafe { GetFileInformationByHandle(handle, &mut information) };
    if result == 0 {
        return Err(format!(
            "{} tanıtıcı üzerinden doğrulanamadı: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    if information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(format!(
            "{} güvenilmeyen bir yeniden yönlendirme noktası.",
            path.display()
        ));
    }
    let is_directory = information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0;
    if is_directory != directory {
        return Err(format!(
            "{} beklenen {} türünde değil.",
            path.display(),
            if directory { "klasör" } else { "dosya" }
        ));
    }
    Ok(())
}

fn open_path(path: &Path, directory: bool, desired_access: u32) -> Result<RawHandle, String> {
    let path_wide = wide_path(path);
    let handle = unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            desired_access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "{} güvenli tanıtıcıyla açılamadı: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    let owned = RawHandle(handle);
    validate_handle_type(owned.0, directory, path)?;
    Ok(owned)
}

fn acl_entries(acl: *mut ACL) -> Result<Vec<*const ACCESS_ALLOWED_ACE>, String> {
    let mut information: ACL_SIZE_INFORMATION = unsafe { zeroed() };
    let read = unsafe {
        GetAclInformation(
            acl,
            (&mut information as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    };
    if read == 0 {
        return Err(format!(
            "Windows erişim listesi ayrıştırılamadı: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut entries = Vec::with_capacity(information.AceCount as usize);
    for index in 0..information.AceCount {
        let mut raw_ace = null_mut();
        let read = unsafe { GetAce(acl, index, &mut raw_ace) };
        if read == 0 || raw_ace.is_null() {
            return Err(format!(
                "Windows erişim kuralı okunamadı: {}",
                std::io::Error::last_os_error()
            ));
        }
        let ace = raw_ace.cast::<ACCESS_ALLOWED_ACE>() as *const ACCESS_ALLOWED_ACE;
        let header = unsafe { &(*ace).Header };
        if header.AceType as u32 != ACCESS_ALLOWED_ACE_TYPE {
            return Err("DNS/servis depolamasında izin verilmeyen erişim kuralı bulundu.".into());
        }
        entries.push(ace);
    }
    Ok(entries)
}

fn ace_sid(ace: *const ACCESS_ALLOWED_ACE) -> PSID {
    unsafe { (&(*ace).SidStart as *const u32).cast_mut().cast() }
}

fn query_descriptor(handle: HANDLE) -> Result<LocalDescriptor, String> {
    let mut descriptor = null_mut();
    let result = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    if result != ERROR_SUCCESS || descriptor.is_null() {
        return Err(format!(
            "Windows erişim güvenliği okunamadı: {}",
            std::io::Error::from_raw_os_error(result as i32)
        ));
    }
    Ok(LocalDescriptor(descriptor))
}

fn verify_handle_acl(handle: HANDLE, directory: bool) -> Result<(), String> {
    let actual = query_descriptor(handle)?;
    let expected = LocalDescriptor::new(directory)?;
    let (actual_owner, actual_dacl) = actual.owner_and_dacl()?;
    let (expected_owner, expected_dacl) = expected.owner_and_dacl()?;

    if unsafe { EqualSid(actual_owner, expected_owner) } == 0 {
        return Err("DNS/servis depolamasının sahibi Administrators değil.".into());
    }

    let mut control = 0;
    let mut revision = 0;
    let control_ok = unsafe { GetSecurityDescriptorControl(actual.0, &mut control, &mut revision) };
    if control_ok == 0 || control & SE_DACL_PROTECTED == 0 {
        return Err("DNS/servis depolamasının erişim listesi korumalı değil.".into());
    }

    let actual_entries = acl_entries(actual_dacl)?;
    let expected_entries = acl_entries(expected_dacl)?;
    if actual_entries.len() != expected_entries.len() {
        return Err("DNS/servis depolamasında beklenmeyen erişim kuralları bulundu.".into());
    }

    for (actual_ace, expected_ace) in actual_entries.iter().zip(expected_entries.iter()) {
        let actual_ace = unsafe { &**actual_ace };
        let expected_ace = unsafe { &**expected_ace };
        if actual_ace.Mask != FILE_ALL_ACCESS
            || actual_ace.Mask != expected_ace.Mask
            || actual_ace.Header.AceFlags != expected_ace.Header.AceFlags
            || actual_ace.Header.AceSize != expected_ace.Header.AceSize
            || unsafe { EqualSid(ace_sid(actual_ace), ace_sid(expected_ace)) } == 0
        {
            return Err("DNS/servis depolamasının erişim kuralları güvenli değil.".into());
        }
    }
    Ok(())
}

fn replace_handle_acl(handle: HANDLE, directory: bool) -> Result<(), String> {
    let expected = LocalDescriptor::new(directory)?;
    let (owner, dacl) = expected.owner_and_dacl()?;
    let security_information: OBJECT_SECURITY_INFORMATION = OWNER_SECURITY_INFORMATION
        | DACL_SECURITY_INFORMATION
        | PROTECTED_DACL_SECURITY_INFORMATION;
    let result = unsafe {
        SetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            security_information,
            owner,
            null_mut(),
            dacl,
            null(),
        )
    };
    if result != ERROR_SUCCESS {
        return Err(format!(
            "Korumalı Windows erişim listesi uygulanamadı: {}",
            std::io::Error::from_raw_os_error(result as i32)
        ));
    }
    verify_handle_acl(handle, directory)
}

pub fn ensure_secure_directory(path: &Path) -> Result<(), String> {
    let descriptor = LocalDescriptor::new(true)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let path_wide = wide_path(path);
    let created = unsafe { CreateDirectoryW(path_wide.as_ptr(), &attributes) };
    if created == 0 {
        let error = unsafe { GetLastError() };
        if error != ERROR_ALREADY_EXISTS {
            return Err(format!(
                "{} güvenli biçimde oluşturulamadı: {}",
                path.display(),
                std::io::Error::from_raw_os_error(error as i32)
            ));
        }
    }

    let handle = open_path(
        path,
        true,
        FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC | WRITE_OWNER,
    )?;
    replace_handle_acl(handle.0, true)
}

pub fn secure_file(path: &Path) -> Result<(), String> {
    let handle = open_path(
        path,
        false,
        FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC | WRITE_OWNER,
    )?;
    replace_handle_acl(handle.0, false)
}

pub fn verify_secure_file(path: &Path) -> Result<(), String> {
    let handle = open_path(path, false, FILE_READ_ATTRIBUTES | READ_CONTROL)?;
    verify_handle_acl(handle.0, false)
}

pub fn create_secure_file(path: &Path, contents: &[u8]) -> Result<(), String> {
    let descriptor = LocalDescriptor::new(false)?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let path_wide = wide_path(path);
    let handle = unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            GENERIC_WRITE | FILE_READ_ATTRIBUTES | READ_CONTROL | WRITE_DAC | WRITE_OWNER,
            FILE_SHARE_READ,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(format!(
            "{} güvenli ve özel olarak oluşturulamadı: {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }

    let owned = RawHandle(handle);
    let result = (|| {
        validate_handle_type(owned.0, false, path)?;
        verify_handle_acl(owned.0, false)?;
        let mut file = owned.into_file();
        file.write_all(contents)
            .map_err(|error| format!("{} yazılamadı: {error}", path.display()))?;
        file.sync_all()
            .map_err(|error| format!("{} diske kaydedilemedi: {error}", path.display()))
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(path);
    }
    result
}
