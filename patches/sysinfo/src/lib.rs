//! The two calls librespot-core makes, with the same answers as sysinfo's.

pub struct System;

#[cfg(not(target_os = "ios"))]
impl System {
    pub fn os_version() -> Option<String> {
        upstream::System::os_version()
    }

    pub fn kernel_version() -> Option<String> {
        upstream::System::kernel_version()
    }
}

#[cfg(target_os = "ios")]
impl System {
    /// "18.1", like sysinfo's.
    pub fn os_version() -> Option<String> {
        sysctl("kern.osproductversion")
    }

    /// The Darwin version, like sysinfo's.
    pub fn kernel_version() -> Option<String> {
        sysctl("kern.osrelease")
    }
}

#[cfg(target_os = "ios")]
fn sysctl(name: &str) -> Option<String> {
    let name = std::ffi::CString::new(name).ok()?;
    let mut buf = [0u8; 64];
    let mut len = buf.len();
    // SAFETY: the name is NUL-terminated and buf/len describe a writable buffer.
    let ok = unsafe {
        libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0)
    } == 0;
    if !ok {
        return None;
    }
    let s = std::ffi::CStr::from_bytes_until_nul(&buf[..len.min(buf.len())]).ok()?;
    Some(s.to_string_lossy().into_owned())
}
