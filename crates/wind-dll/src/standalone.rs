use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::{Foundation::*, System::Threading::*};

static READY: AtomicUsize = AtomicUsize::new(0);

/// Keep an event alive in the DLL, so a restarted watcher can observe readiness.
pub fn signal_ready() -> Result<(), String> {
    unsafe {
        let mut created: FILETIME = std::mem::zeroed();
        let mut exited: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        if GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        ) == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let time = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        let name: Vec<u16> = wind_common::locale::ready_event(GetCurrentProcessId(), time)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let event = CreateEventW(std::ptr::null(), 1, 0, name.as_ptr());
        if event.is_null() {
            return Err(std::io::Error::last_os_error().to_string());
        }
        if SetEvent(event) == 0 {
            let error = std::io::Error::last_os_error().to_string();
            CloseHandle(event);
            return Err(error);
        }
        READY.store(event as usize, Ordering::Release);
    }
    Ok(())
}
