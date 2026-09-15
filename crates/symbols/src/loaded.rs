//! Read identity from the loaded image, not from a potentially replaced file.

use crate::image::{Headers, Identity};
use std::ffi::c_void;
use windows_sys::Win32::System::{
    Diagnostics::Debug::ReadProcessMemory, LibraryLoader::*, Memory::*,
    Threading::GetCurrentProcess,
};

pub struct LoadedModule {
    pub base: usize,
    pub headers: Headers,
    pub identity: Identity,
}

impl Drop for LoadedModule {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::FreeLibrary(self.base as *mut c_void);
        }
    }
}

fn read(base: usize, rva: u32, len: u32) -> Result<Vec<u8>, String> {
    let address = base.checked_add(rva as usize).ok_or("address overflow")?;
    let mut data = vec![0u8; len as usize];
    let mut read = 0;
    let ok = unsafe {
        ReadProcessMemory(
            GetCurrentProcess(),
            address as *const c_void,
            data.as_mut_ptr().cast(),
            data.len(),
            &mut read,
        )
    };
    if ok == 0 || read != data.len() {
        return Err(format!(
            "read loaded PE: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(data)
}

impl LoadedModule {
    pub fn find(name: &str) -> Result<Option<Self>, String> {
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        let mut handle = std::ptr::null_mut();
        // Take a reference so resolution and the installed hook cannot outlive the module.
        if unsafe { GetModuleHandleExW(0, wide.as_ptr(), &mut handle) } == 0 {
            return Ok(None);
        }
        let base = handle as usize;
        let result = (|| {
            let dos = read(base, 0, 64)?;
            if &dos[..2] != b"MZ" {
                return Err("invalid loaded DOS header".into());
            }
            let pe = u32::from_le_bytes(dos[60..64].try_into().unwrap());
            if pe > 1024 * 1024 {
                return Err("invalid loaded PE offset".into());
            }
            let nt = read(base, pe, 24)?;
            let count = u16::from_le_bytes(nt[6..8].try_into().unwrap()) as u32;
            let optional = u16::from_le_bytes(nt[20..22].try_into().unwrap()) as u32;
            if count > 96 || optional > 4096 {
                return Err("oversized loaded PE headers".into());
            }
            let data = read(base, 0, pe + 24 + optional + count * 40)?;
            let headers = Headers::parse(&data)?;
            let identity = headers.identity(|rva, len| read(base, rva, len))?;
            Ok(Some(Self {
                base,
                headers,
                identity,
            }))
        })();
        if result.is_err() {
            unsafe {
                windows_sys::Win32::Foundation::FreeLibrary(handle);
            }
        }
        result
    }

    pub fn function_address(&self, rva: u32) -> Result<usize, String> {
        if !self.headers.executable(rva) {
            return Err("RVA is not executable".into());
        }
        let address = self
            .base
            .checked_add(rva as usize)
            .ok_or("address overflow")?;
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let len = unsafe {
            VirtualQuery(
                address as *const c_void,
                &mut info,
                std::mem::size_of_val(&info),
            )
        };
        let executable =
            PAGE_EXECUTE | PAGE_EXECUTE_READ | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
        if len == 0
            || info.State != MEM_COMMIT
            || info.Protect & executable == 0
            || info.Protect & (PAGE_GUARD | PAGE_NOACCESS) != 0
            || info.AllocationBase as usize != self.base
        {
            return Err("resolved function is not in committed executable module memory".into());
        }
        Ok(address)
    }
}
