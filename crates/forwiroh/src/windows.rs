//! Owner-only Windows security for the state directory and local control pipe.
use std::{io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
        DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SECURITY_ATTRIBUTES,
        SetFileSecurityW,
    },
};

pub struct OwnerSecurity(*mut core::ffi::c_void);

impl OwnerSecurity {
    pub fn new() -> io::Result<Self> {
        // Owner Rights and SYSTEM only; child files/directories inherit this ACL.
        let sddl: Vec<u16> = "D:P(A;OICI;GA;;;OW)(A;OICI;GA;;;SY)\0"
            .encode_utf16()
            .collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: SDDL is NUL terminated and descriptor is a valid output slot.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(descriptor))
    }

    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }

    pub fn apply(&self, path: &Path) -> io::Result<()> {
        let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SAFETY: path is NUL terminated; the descriptor lives through this call.
        if unsafe {
            SetFileSecurityW(
                path.as_ptr(),
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                self.0,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for OwnerSecurity {
    fn drop(&mut self) {
        // SAFETY: the conversion allocated this descriptor with LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}
