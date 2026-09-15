//! DbgHelp runs only in the resolver process. No networking or symbol loading in Explorer.

use crate::{
    image::{Headers, Identity},
    Cache, Symbol, BUILD_SYMBOL, UPDATE_SYMBOL,
};
use std::{
    ffi::{c_void, CString},
    path::{Path, PathBuf},
    sync::Mutex,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Diagnostics::Debug::*, LibraryLoader::*, Threading::GetCurrentProcess},
};

static DBGHELP_LOCK: Mutex<()> = Mutex::new(());
type Initialize = unsafe extern "system" fn(HANDLE, *const u16, i32) -> i32;
type Cleanup = unsafe extern "system" fn(HANDLE) -> i32;
type Options = unsafe extern "system" fn(u32) -> u32;
type Load = unsafe extern "system" fn(
    HANDLE,
    HANDLE,
    *const u16,
    *const u16,
    u64,
    u32,
    *const c_void,
    u32,
) -> u64;
type Info = unsafe extern "system" fn(HANDLE, u64, *mut IMAGEHLP_MODULE64) -> i32;
type Lookup = unsafe extern "system" fn(HANDLE, *const u8, *mut SYMBOL_INFO) -> i32;

struct Library(HMODULE);
impl Drop for Library {
    fn drop(&mut self) {
        unsafe {
            FreeLibrary(self.0);
        }
    }
}
impl Library {
    fn load(path: &Path) -> Result<Self, String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let wide: Vec<u16> = path
            .as_os_str()
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let h = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS,
            )
        };
        if h.is_null() {
            return Err(format!(
                "load {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
        Ok(Self(h))
    }
    unsafe fn proc(&self, name: &[u8]) -> Result<unsafe extern "system" fn() -> isize, String> {
        GetProcAddress(self.0, name.as_ptr())
            .ok_or_else(|| format!("DbgHelp export missing: {}", String::from_utf8_lossy(name)))
    }
}

pub fn default_dbghelp() -> PathBuf {
    // The runtime bundle declares its debugger DLLs. Explicit --dbghelp remains
    // available for development; never depend on a developer's SDK by default.
    std::env::current_exe()
        .expect("current executable path")
        .with_file_name("dbghelp.dll")
}

struct Session {
    process: HANDLE,
    cleanup: Cleanup,
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            (self.cleanup)(self.process);
        }
    }
}

pub fn resolve(
    image: &Path,
    module: &str,
    headers: &Headers,
    identity: &Identity,
    symbol_path: &str,
    dbghelp: &Path,
) -> Result<Cache, String> {
    let _lock = DBGHELP_LOCK.lock().map_err(|_| "DbgHelp lock poisoned")?;
    // Keep SDK symsrv beside SDK dbghelp, avoiding an incompatible system copy.
    let symsrv = dbghelp.with_file_name("symsrv.dll");
    let _symsrv = if symsrv.is_file() {
        Some(Library::load(&symsrv)?)
    } else {
        None
    };
    let library = Library::load(dbghelp)?;
    let (init, cleanup, options, load, info, lookup): (
        Initialize,
        Cleanup,
        Options,
        Load,
        Info,
        Lookup,
    ) = unsafe {
        (
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Initialize>(
                library.proc(b"SymInitializeW\0")?,
            ),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Cleanup>(
                library.proc(b"SymCleanup\0")?,
            ),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Options>(
                library.proc(b"SymSetOptions\0")?,
            ),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Load>(
                library.proc(b"SymLoadModuleExW\0")?,
            ),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Info>(
                library.proc(b"SymGetModuleInfo64\0")?,
            ),
            std::mem::transmute::<unsafe extern "system" fn() -> isize, Lookup>(
                library.proc(b"SymFromName\0")?,
            ),
        )
    };
    let process = unsafe { GetCurrentProcess() };
    // Exact PDB matching; deliberately do not enable LOAD_ANYTHING or UNDNAME.
    unsafe {
        options(
            SYMOPT_EXACT_SYMBOLS
                | SYMOPT_FAIL_CRITICAL_ERRORS
                | SYMOPT_NO_PROMPTS
                | SYMOPT_IGNORE_NT_SYMPATH,
        );
    }
    let search: Vec<u16> = symbol_path.encode_utf16().chain(Some(0)).collect();
    if unsafe { init(process, search.as_ptr(), 0) } == 0 {
        return Err(format!(
            "SymInitialize: {}",
            std::io::Error::last_os_error()
        ));
    }
    let _session = Session { process, cleanup };
    let image_wide: Vec<u16> = image
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let alias: Vec<u16> = "taskbar_target\0".encode_utf16().collect();
    // A synthetic base for offline symbol loading. The DLL is never executed here.
    let base = unsafe {
        load(
            process,
            std::ptr::null_mut(),
            image_wide.as_ptr(),
            alias.as_ptr(),
            0x10000000,
            headers.image_size,
            std::ptr::null(),
            0,
        )
    };
    if base == 0 {
        return Err(format!(
            "SymLoadModuleEx: {}",
            std::io::Error::last_os_error()
        ));
    }
    let mut module_info = IMAGEHLP_MODULE64 {
        SizeOfStruct: std::mem::size_of::<IMAGEHLP_MODULE64>() as u32,
        ..Default::default()
    };
    if unsafe { info(process, base, &mut module_info) } == 0 {
        return Err(format!(
            "SymGetModuleInfo64: {}",
            std::io::Error::last_os_error()
        ));
    }
    let guid: [u8; 16] = unsafe { std::mem::transmute(module_info.PdbSig70) };
    if module_info.SymType != SymPdb
        || module_info.PdbUnmatched != 0
        || module_info.DbgUnmatched != 0
        || guid != identity.pdb_guid
        || module_info.PdbAge != identity.pdb_age
        || module_info.ImageSize != identity.image_size
        || module_info.TimeDateStamp != identity.timestamp
    {
        return Err("DbgHelp did not load a PDB matching this exact image".into());
    }
    let find = |name: &str| -> Result<Symbol, String> {
        // u64 storage supplies SYMBOL_INFO alignment and space for the trailing name.
        let mut storage = vec![0u64; (std::mem::size_of::<SYMBOL_INFO>() + 2048).div_ceil(8)];
        let ptr = storage.as_mut_ptr().cast::<SYMBOL_INFO>();
        unsafe {
            (*ptr).SizeOfStruct = std::mem::size_of::<SYMBOL_INFO>() as u32;
            (*ptr).MaxNameLen = 2048;
        }
        let query = CString::new(format!("taskbar_target!{name}")).map_err(|e| e.to_string())?;
        if unsafe { lookup(process, query.as_ptr().cast(), ptr) } == 0 {
            return Err(format!(
                "symbol {name} not found: {}",
                std::io::Error::last_os_error()
            ));
        }
        let symbol = unsafe { &*ptr };
        if symbol.NameLen >= 2048 {
            return Err("oversized symbol name".into());
        }
        let actual = unsafe {
            std::slice::from_raw_parts(symbol.Name.as_ptr().cast::<u8>(), symbol.NameLen as usize)
        };
        // Microsoft's stripped public PDBs mark functions PUBLIC_CODE, not FUNCTION.
        if actual != name.as_bytes()
            || symbol.ModBase != base
            || symbol.Flags & (SYMFLAG_FUNCTION | SYMFLAG_PUBLIC_CODE) == 0
        {
            return Err(format!("symbol lookup mismatch for {name}: name={}, base=0x{:X}, expected_base=0x{base:X}, flags=0x{:X}", String::from_utf8_lossy(actual), symbol.ModBase, symbol.Flags));
        }
        let rva = symbol
            .Address
            .checked_sub(base)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("symbol address outside module")?;
        Ok(Symbol {
            name: name.into(),
            rva,
        })
    };
    let cache = Cache {
        schema: 1,
        module: module.into(),
        identity: identity.clone(),
        build: find(BUILD_SYMBOL)?,
        update: find(UPDATE_SYMBOL)?,
    };
    cache.validate(module, headers, identity)?;
    // The image could have been serviced while symbols were downloading.
    if crate::image::inspect_file(image)?.1 != *identity {
        return Err("image changed during resolution; retry".into());
    }
    Ok(cache)
}
