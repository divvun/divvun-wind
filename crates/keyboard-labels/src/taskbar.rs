use std::{
    ffi::c_void,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Mutex,
    },
};
use wind_symbols::{loaded::LoadedModule, MODULES};

use crate::debug_log;

// Both decorated symbols describe void instance methods, with only `this`.
type InstanceMethod = unsafe extern "system" fn(*mut c_void);
static mut ORIGINAL_UPDATE: *mut c_void = std::ptr::null_mut();
static BUILD_ADDRESS: AtomicUsize = AtomicUsize::new(0);
static REBUILD_CLAIMED: AtomicBool = AtomicBool::new(false);

struct Installed {
    // Keep the target DLL referenced even after detaching: a callback may still be running.
    _module: Option<LoadedModule>,
    attached: bool,
}
static INSTALLED: Mutex<Option<Installed>> = Mutex::new(None);

unsafe extern "system" fn hook_update_current_data(this: *mut c_void) {
    if !REBUILD_CLAIMED.swap(true, Ordering::AcqRel) {
        // Detach before rebuilding: rebuilding may cause another UpdateCurrentData call.
        match uninstall_rebuild_hook() {
            Ok(()) => {
                let build: InstanceMethod =
                    std::mem::transmute(BUILD_ADDRESS.load(Ordering::Acquire));
                debug_log("rebuilding the enabled language list");
                build(this);
                debug_log("enabled language list rebuilt");
            }
            Err(e) => {
                debug_log(&format!("cannot detach refresh hook: {e}"));
                REBUILD_CLAIMED.store(false, Ordering::Release);
            }
        }
    }
    let original: InstanceMethod = std::mem::transmute(ORIGINAL_UPDATE);
    original(this);
}

pub(crate) fn own_directory(hinst: usize) -> Result<PathBuf, String> {
    let mut path = vec![0u16; 32768];
    let len = unsafe {
        windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(
            hinst as *mut c_void,
            path.as_mut_ptr(),
            path.len() as u32,
        )
    } as usize;
    if len == 0 || len >= path.len() {
        return Err("cannot get locale DLL directory".into());
    }
    PathBuf::from(String::from_utf16_lossy(&path[..len]))
        .parent()
        .map(|p| p.to_path_buf())
        .ok_or("DLL has no parent directory".into())
}

/// Install using an exact-image cache, without loading DbgHelp or using the network.
pub fn install_rebuild_hook(hinst: usize) -> Result<(), String> {
    let result = install_from_cache(hinst);
    if let Err(e) = &result {
        debug_log(e);
    }
    result
}

fn install_from_cache(hinst: usize) -> Result<(), String> {
    let roots = [
        own_directory(hinst)?.join("taskbar-symbols"),
        wind_symbols::default_cache_dir(),
    ];
    let mut errors = Vec::new();
    for name in MODULES {
        let module = match LoadedModule::find(name) {
            Ok(Some(module)) => module,
            Ok(None) => {
                errors.push(format!("{name} is not loaded"));
                continue;
            }
            Err(e) => {
                errors.push(format!("{name}: {e}"));
                continue;
            }
        };
        for root in &roots {
            let cache =
                match wind_symbols::read_cache(root, name, &module.headers, &module.identity) {
                    Ok(cache) => cache,
                    Err(e) => {
                        errors.push(e);
                        continue;
                    }
                };
            let update = module.function_address(cache.update.rva)?;
            let build = module.function_address(cache.build.rva)?;
            debug_log(&format!("validated {name}, identity={}, UpdateCurrentData=0x{update:X}, BuildEnabledLanguagesList=0x{build:X}", module.identity.key()));
            unsafe {
                install_targets(Some(module), update, build)?;
            }
            debug_log("symbol-resolved refresh hook installed; waiting for UpdateCurrentData");
            return Ok(());
        }
    }
    let error = format!(
        "no matching symbol cache; run divvun-wind-symbols prepare before injection:\n{}",
        errors.join("\n")
    );
    Err(error)
}

unsafe fn install_targets(
    module: Option<LoadedModule>,
    update: usize,
    build: usize,
) -> Result<(), String> {
    let mut installed = INSTALLED.lock().map_err(|_| "refresh-hook lock poisoned")?;
    if installed.as_ref().is_some_and(|s| s.attached) {
        return Err("refresh hook already installed".into());
    }
    ORIGINAL_UPDATE = update as *mut c_void;
    BUILD_ADDRESS.store(build, Ordering::Release);
    REBUILD_CLAIMED.store(false, Ordering::Release);
    let mut txn = windows_detours::Transaction::new().map_err(|e| e.to_string())?;
    txn.update_current_thread().map_err(|e| e.to_string())?;
    let hook = windows_detours::RawDetourHook::new(
        &raw mut ORIGINAL_UPDATE,
        hook_update_current_data as *mut c_void,
    );
    txn.attach_raw(&hook).map_err(|e| e.to_string())?;
    txn.commit().map_err(|e| e.to_string())?;
    *installed = Some(Installed {
        _module: module,
        attached: true,
    });
    Ok(())
}

/// Also used before ejection if the one-time refresh has not fired yet.
pub fn uninstall_rebuild_hook() -> Result<(), String> {
    let mut installed = INSTALLED.lock().map_err(|_| "refresh-hook lock poisoned")?;
    let Some(state) = installed.as_mut().filter(|s| s.attached) else {
        return Ok(());
    };
    let mut txn = windows_detours::Transaction::new().map_err(|e| e.to_string())?;
    txn.update_current_thread().map_err(|e| e.to_string())?;
    unsafe {
        let hook = windows_detours::RawDetourHook::new(
            &raw mut ORIGINAL_UPDATE,
            hook_update_current_data as *mut c_void,
        );
        txn.detach_raw(&hook).map_err(|e| e.to_string())?;
    }
    txn.commit().map_err(|e| e.to_string())?;
    state.attached = false;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[repr(C)]
    struct Counts {
        updates: usize,
        builds: usize,
    }
    #[inline(never)]
    unsafe extern "system" fn update(this: *mut c_void) {
        let counts = &mut *std::hint::black_box(this.cast::<Counts>());
        counts.updates += 1;
        std::hint::black_box(counts.updates);
    }
    #[inline(never)]
    unsafe extern "system" fn build(this: *mut c_void) {
        let counts = &mut *this.cast::<Counts>();
        counts.builds += 1;
    }
    #[test]
    fn detour_rebuilds_once_and_can_be_removed_before_it_fires() {
        let _transaction_lock = crate::DETOUR_TEST_LOCK.lock().unwrap();
        let mut counts = Counts {
            updates: 0,
            builds: 0,
        };
        unsafe {
            install_targets(
                None,
                update as *const () as usize,
                build as *const () as usize,
            )
            .unwrap();
            let call: InstanceMethod = std::hint::black_box(update);
            call((&mut counts as *mut Counts).cast());
            call((&mut counts as *mut Counts).cast());
            assert_eq!((counts.updates, counts.builds), (2, 1));
            install_targets(
                None,
                update as *const () as usize,
                build as *const () as usize,
            )
            .unwrap();
            uninstall_rebuild_hook().unwrap();
            call((&mut counts as *mut Counts).cast());
            assert_eq!((counts.updates, counts.builds), (3, 1));
        }
    }
}
