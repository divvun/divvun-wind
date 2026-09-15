#![cfg_attr(not(test), windows_subsystem = "windows")]

use std::{io, os::windows::process::CommandExt, process::Command};
use windows_sys::Win32::{
    System::Threading::CREATE_NO_WINDOW,
    UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK},
};

#[cfg(feature = "embedded-payload")]
const PAYLOAD: &[u8] = include_bytes!(env!("WIND_OUTTO_INSTALLER"));
#[cfg(not(feature = "embedded-payload"))]
const PAYLOAD: &[u8] = &[];

fn guarded_install(eligible: bool, install: impl FnOnce() -> io::Result<i32>) -> io::Result<i32> {
    if !eligible {
        return Ok(0);
    }
    install()
}

fn install() -> io::Result<i32> {
    if !PAYLOAD.starts_with(b"MZ") {
        return Err(io::Error::other(
            "This build has no embedded outto installer",
        ));
    }
    // This directory is created only after the shared Windows 11 eligibility
    // check. Keep the outto installer alive until its child process completes.
    let stage = tempfile::Builder::new()
        .prefix("divvun-wind-install-")
        .tempdir()?;
    let installer = stage.path().join("outto-installer.exe");
    std::fs::write(&installer, PAYLOAD)?;
    let status = Command::new(&installer)
        .args(std::env::args_os().skip(1))
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;
    Ok(status.code().unwrap_or(1))
}

fn main() {
    // Before argument handling, extraction, installer UI, or UAC elevation.
    let result = guarded_install(wind_common::os::eligible(), install);
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            let title: Vec<u16> = "Divvun Wind\0".encode_utf16().collect();
            let text: Vec<u16> = format!("Could not start the installer: {error}\0")
                .encode_utf16()
                .collect();
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_os_skips_all_installer_work() {
        assert_eq!(
            guarded_install(false, || panic!("must not extract or launch")).unwrap(),
            0
        );
    }

    #[test]
    fn installer_exit_status_and_launch_errors_are_preserved() {
        assert_eq!(guarded_install(true, || Ok(3010)).unwrap(), 3010);
        let error = guarded_install(true, || Err(io::Error::other("launch failed"))).unwrap_err();
        assert_eq!(error.to_string(), "launch failed");
    }
}
