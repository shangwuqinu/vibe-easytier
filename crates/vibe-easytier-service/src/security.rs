//! Windows ACL helpers for service-owned durable data.

use std::{io, path::Path};

/// Applies a protected DACL that leaves the encrypted state and generated
/// runtime configuration accessible only to LocalSystem and Administrators.
/// The desktop app must use the service pipe rather than reading these files.
#[cfg(all(windows, not(test)))]
pub fn harden_service_path(path: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let descriptor = SecurityDescriptor::from_sddl(SERVICE_DATA_SDDL)?;
    let success = unsafe {
        SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION,
            descriptor.as_ptr(),
        )
    };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(any(not(windows), test))]
pub fn harden_service_path(_path: &Path) -> io::Result<()> {
    // Unit tests run as the interactive build account, not LocalSystem. A
    // production DACL would intentionally lock that account out of its own
    // temporary fixture after the first write. Installer and service tests
    // exercise the real ACL path on a deployed service.
    Ok(())
}

#[cfg(all(windows, not(test)))]
const SERVICE_DATA_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)";
#[cfg(all(windows, not(test)))]
const SDDL_REVISION_1: u32 = 1;
#[cfg(all(windows, not(test)))]
const DACL_SECURITY_INFORMATION: u32 = 0x0000_0004;

#[cfg(all(windows, not(test)))]
struct SecurityDescriptor(*mut std::ffi::c_void);

#[cfg(all(windows, not(test)))]
impl SecurityDescriptor {
    fn from_sddl(sddl: &str) -> io::Result<Self> {
        use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

        let sddl: Vec<u16> = OsStr::new(sddl).encode_wide().chain(Some(0)).collect();
        let mut descriptor = std::ptr::null_mut();
        let created = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if created == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }

    fn as_ptr(&self) -> *mut std::ffi::c_void {
        self.0
    }
}

#[cfg(all(windows, not(test)))]
impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}

#[cfg(all(windows, not(test)))]
#[link(name = "advapi32")]
extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        string_security_descriptor: *const u16,
        revision: u32,
        security_descriptor: *mut *mut std::ffi::c_void,
        security_descriptor_size: *mut u32,
    ) -> i32;
    fn SetFileSecurityW(
        file_name: *const u16,
        security_information: u32,
        security_descriptor: *mut std::ffi::c_void,
    ) -> i32;
}

#[cfg(all(windows, not(test)))]
#[link(name = "kernel32")]
extern "system" {
    fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
}
