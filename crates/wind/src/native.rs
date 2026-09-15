use std::{
    ffi::c_void,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::SYNCHRONIZE,
    System::{
        Diagnostics::{Debug::*, ToolHelp::*},
        LibraryLoader::*,
        Memory::*,
        RemoteDesktop::ProcessIdToSessionId,
        Threading::*,
    },
    UI::WindowsAndMessaging::*,
};

pub fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn error(context: &str) -> String {
    format!("{context}: {}", std::io::Error::last_os_error())
}

pub struct Handle(pub HANDLE);
impl Handle {
    pub fn checked(value: HANDLE, context: &str) -> Result<Self, String> {
        if value.is_null() || value == INVALID_HANDLE_VALUE {
            Err(error(context))
        } else {
            Ok(Self(value))
        }
    }
    pub fn signaled(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_OBJECT_0 }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn session_id() -> Result<u32, String> {
    let mut session = 0;
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(error("session"));
    }
    Ok(session)
}

pub struct Process {
    pub handle: Handle,
    pub pid: u32,
    pub created: u64,
}
impl Process {
    pub fn open(pid: u32, inject: bool) -> Result<Self, String> {
        let mut access = SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
        if inject {
            access |= PROCESS_CREATE_THREAD
                | PROCESS_VM_OPERATION
                | PROCESS_VM_WRITE
                | PROCESS_VM_READ
                | PROCESS_QUERY_INFORMATION;
        }
        let handle = Handle::checked(unsafe { OpenProcess(access, 0, pid) }, "OpenProcess")?;
        let created = creation_time(handle.0)?;
        Ok(Self {
            handle,
            pid,
            created,
        })
    }
    pub fn path(&self) -> Result<PathBuf, String> {
        let mut buffer = vec![0u16; 32768];
        let mut len = buffer.len() as u32;
        if unsafe { QueryFullProcessImageNameW(self.handle.0, 0, buffer.as_mut_ptr(), &mut len) }
            == 0
        {
            return Err(error("process image"));
        }
        Ok(PathBuf::from(String::from_utf16_lossy(
            &buffer[..len as usize],
        )))
    }
    pub fn ready(&self) -> bool {
        let name = wide(wind_common::locale::ready_event(self.pid, self.created));
        Handle::checked(
            unsafe { OpenEventW(SYNCHRONIZE, 0, name.as_ptr()) },
            "ready event",
        )
        .is_ok_and(|event| event.signaled())
    }
}

pub fn creation_time(process: HANDLE) -> Result<u64, String> {
    unsafe {
        let mut created: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        if GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user) == 0 {
            return Err(error("process creation time"));
        }
        Ok(((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64)
    }
}

/// The desktop shell and taskbar must agree. Never target arbitrary Explorer windows.
pub fn shell(session: u32) -> Result<Option<Process>, String> {
    unsafe {
        let desktop = GetShellWindow();
        let tray = FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null());
        if desktop.is_null() || tray.is_null() {
            return Ok(None);
        }
        let (mut pid, mut tray_pid, mut target_session) = (0, 0, 0);
        GetWindowThreadProcessId(desktop, &mut pid);
        GetWindowThreadProcessId(tray, &mut tray_pid);
        if pid == 0 || pid != tray_pid {
            return Ok(None);
        }
        if ProcessIdToSessionId(pid, &mut target_session) == 0 {
            return Err(error("shell session"));
        }
        if target_session != session {
            return Err("shell is in a different session".into());
        }
        let process = Process::open(pid, true)?;
        let expected = PathBuf::from(std::env::var_os("SystemRoot").ok_or("SystemRoot is unset")?)
            .join("explorer.exe");
        if !same_path(&process.path()?, &expected) {
            return Err("desktop shell is not Windows Explorer".into());
        }
        Ok(Some(process))
    }
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy()),
        _ => false,
    }
}

#[derive(Debug)]
pub struct Module {
    pub name: String,
    pub path: PathBuf,
    pub base: usize,
    pub size: u32,
}
pub fn modules(pid: u32) -> Result<Vec<Module>, String> {
    unsafe {
        let mut snapshot = INVALID_HANDLE_VALUE;
        for _ in 0..10 {
            snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
            if snapshot != INVALID_HANDLE_VALUE || GetLastError() != ERROR_BAD_LENGTH {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let snapshot = Handle::checked(snapshot, "module snapshot")?;
        let mut entry: MODULEENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        if Module32FirstW(snapshot.0, &mut entry) == 0 {
            return Err(error("first module"));
        }
        let mut result = Vec::new();
        loop {
            let string = |buf: &[u16]| {
                String::from_utf16_lossy(
                    &buf[..buf.iter().position(|&c| c == 0).unwrap_or(buf.len())],
                )
            };
            result.push(Module {
                name: string(&entry.szModule),
                path: PathBuf::from(string(&entry.szExePath)),
                base: entry.modBaseAddr as usize,
                size: entry.modBaseSize,
            });
            if Module32NextW(snapshot.0, &mut entry) == 0 {
                if GetLastError() != ERROR_NO_MORE_FILES {
                    return Err(error("next module"));
                }
                break;
            }
        }
        Ok(result)
    }
}

/// Resolve the exported loader in its actual containing module, then relocate
/// to that module in the target. This also handles forwarded Kernel32 exports.
fn remote_loader(target: &[Module]) -> Result<usize, String> {
    unsafe {
        let kernel = GetModuleHandleW(wide("kernel32.dll").as_ptr());
        let loader = GetProcAddress(kernel, c"LoadLibraryW".as_ptr().cast())
            .ok_or("LoadLibraryW missing")?;
        let mut owner = std::ptr::null_mut();
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            loader as *const u16,
            &mut owner,
        ) == 0
        {
            return Err(error("loader module"));
        }
        let mut name = vec![0u16; 32768];
        let len = GetModuleFileNameW(owner, name.as_mut_ptr(), name.len() as u32);
        if len == 0 || len as usize >= name.len() {
            return Err(error("loader module path"));
        }
        let path = PathBuf::from(String::from_utf16_lossy(&name[..len as usize]));
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or("loader module filename")?;
        let local =
            wind_symbols::loaded::LoadedModule::find(name)?.ok_or("loader module disappeared")?;
        let remote = target
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
            .ok_or("loader module not loaded in target")?;
        if wind_symbols::image::inspect_file(&remote.path)?.1 != local.identity
            || remote.size != local.headers.image_size
        {
            return Err("local and target loader module versions differ".into());
        }
        let rva = (loader as usize)
            .checked_sub(local.base)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or("loader address outside module")?;
        local.function_address(rva)?;
        remote
            .base
            .checked_add(rva as usize)
            .ok_or("remote loader address overflow".into())
    }
}

pub fn inject(process: &Process, dll: &Path) -> Result<(), String> {
    let target = modules(process.pid)?;
    if target
        .iter()
        .any(|m| m.name.eq_ignore_ascii_case("divvun_keyboard_labels.dll"))
    {
        return Ok(());
    }
    let loader = remote_loader(&target)?;
    let path = wide(dll.canonicalize().map_err(|e| e.to_string())?);
    let bytes = path.len() * 2;
    unsafe {
        let remote = VirtualAllocEx(
            process.handle.0,
            std::ptr::null(),
            bytes,
            MEM_COMMIT | MEM_RESERVE,
            PAGE_READWRITE,
        );
        if remote.is_null() {
            return Err(error("allocate DLL path"));
        }
        let mut written = 0;
        if WriteProcessMemory(
            process.handle.0,
            remote,
            path.as_ptr().cast(),
            bytes,
            &mut written,
        ) == 0
            || written != bytes
        {
            let e = error("write DLL path");
            VirtualFreeEx(process.handle.0, remote, 0, MEM_RELEASE);
            return Err(e);
        }
        let thread = CreateRemoteThread(
            process.handle.0,
            std::ptr::null(),
            0,
            Some(std::mem::transmute::<
                usize,
                unsafe extern "system" fn(*mut c_void) -> u32,
            >(loader)),
            remote,
            0,
            std::ptr::null_mut(),
        );
        if thread.is_null() {
            let e = error("create loader thread");
            VirtualFreeEx(process.handle.0, remote, 0, MEM_RELEASE);
            return Err(e);
        }
        let thread = Handle(thread);
        if WaitForSingleObject(thread.0, 10000) != WAIT_OBJECT_0 {
            // The loader may still be reading the buffer. Retain it until process
            // exit; the watcher never starts a second load attempt in this process.
            return Err(
                "loader has not completed; retaining remote path and waiting for Explorer restart"
                    .into(),
            );
        }
        VirtualFreeEx(process.handle.0, remote, 0, MEM_RELEASE);
    }
    Ok(())
}

/// True means cancellation or process exit. A healthy process costs no CPU while waiting.
pub fn wait(stop: &Handle, process: Option<&Process>, milliseconds: u32) -> Result<bool, String> {
    let handles = [stop.0, process.map_or(std::ptr::null_mut(), |p| p.handle.0)];
    let count = if process.is_some() { 2 } else { 1 };
    let result = unsafe { WaitForMultipleObjects(count, handles.as_ptr(), 0, milliseconds) };
    match result {
        WAIT_TIMEOUT => Ok(false),
        WAIT_FAILED => Err(error("wait")),
        _ => Ok(true),
    }
}
