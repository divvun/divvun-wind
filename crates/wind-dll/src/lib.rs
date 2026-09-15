//! Keyboard label capability for Divvun Wind. Loaded once per Explorer lifetime.
mod config;
mod hooks;
mod standalone;
mod taskbar;
mod ui_language;

#[cfg(test)]
static DETOUR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

use std::ffi::c_void;

// Diagnostics go to an attached debugger. Hook callbacks never open log files.
fn debug_log(message: &str) {
    let text: Vec<u16> = format!("Divvun Wind keyboard labels: {message}\n")
        .encode_utf16()
        .chain([0])
        .collect();
    unsafe {
        windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW(text.as_ptr());
    }
}

/// # Safety
/// Called by the Windows loader with its documented DLL entry point arguments.
#[no_mangle]
pub unsafe extern "system" fn DllMain(
    hinst: *mut c_void,
    reason: u32,
    _reserved: *mut c_void,
) -> i32 {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::{LibraryLoader::*, Threading::CreateThread},
    };
    if reason != 1 || !wind_common::os::eligible() {
        return 1;
    }
    // Pin before starting a worker: hooks and any in-flight callback must remain
    // mapped for the process lifetime. No detach-time locks, joins or Detours.
    let mut pinned = std::ptr::null_mut();
    if GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
        hinst.cast(),
        &mut pinned,
    ) == 0
    {
        return 1;
    }
    let thread = CreateThread(
        std::ptr::null(),
        0,
        Some(initialize),
        hinst,
        0,
        std::ptr::null_mut(),
    );
    if !thread.is_null() {
        CloseHandle(thread);
    }
    1
}

unsafe extern "system" fn initialize(hinst: *mut c_void) -> u32 {
    if let Err(error) = initialize_labels(hinst as usize) {
        debug_log(&error);
    }
    0
}

fn initialize_labels(hinst: usize) -> Result<(), String> {
    let snapshot = config::load_config()?;
    for diagnostic in &snapshot.diagnostics {
        debug_log(&format!("{diagnostic:?}"));
    }
    if !snapshot.scan_complete {
        return Err("catalogue scan unavailable".into());
    }
    if !snapshot.entries.is_empty() {
        hooks::install_hooks(snapshot.entries)?;
        taskbar::install_rebuild_hook(hinst)?;
    }
    // Empty is a successfully initialized pass-through snapshot. Readiness
    // acknowledges initialization; it does not claim the UI refresh has fired.
    standalone::signal_ready()
}
