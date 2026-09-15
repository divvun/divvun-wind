use crate::config::LocaleConfig;
use std::ffi::c_void;
use std::sync::OnceLock;

type Hresult = i32;
const S_OK: Hresult = 0;

use crate::debug_log;

type FnBcp47GetAbbreviation = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult;
type FnWindowsCreateString =
    unsafe extern "system" fn(*const u16, u32, *mut *mut c_void) -> Hresult;
type FnWindowsGetStringRawBuffer = unsafe extern "system" fn(*mut c_void, *mut u32) -> *const u16;

/// Original function pointer — Detours overwrites this with the trampoline.
static mut ORIG_BCP47_GET_ABBREVIATION: *mut c_void = std::ptr::null_mut();

/// Resolved WindowsCreateString from combase.dll.
static WCS: OnceLock<Option<FnWindowsCreateString>> = OnceLock::new();
static WGSRB: OnceLock<Option<FnWindowsGetStringRawBuffer>> = OnceLock::new();

fn get_wcs() -> Option<FnWindowsCreateString> {
    *WCS.get_or_init(|| unsafe {
        let name: Vec<u16> = "combase.dll\0".encode_utf16().collect();
        let module = windows_sys::Win32::System::LibraryLoader::LoadLibraryW(name.as_ptr());
        if module.is_null() {
            return None;
        }
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            module,
            c"WindowsCreateString".as_ptr().cast(),
        )
        .map(|f| {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, FnWindowsCreateString>(f)
        })
    })
}

fn get_wgsrb() -> Option<FnWindowsGetStringRawBuffer> {
    *WGSRB.get_or_init(|| unsafe {
        let name: Vec<u16> = "combase.dll\0".encode_utf16().collect();
        let module = windows_sys::Win32::System::LibraryLoader::LoadLibraryW(name.as_ptr());
        if module.is_null() { return None; }
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(module, c"WindowsGetStringRawBuffer".as_ptr().cast())
            .map(|f| std::mem::transmute::<unsafe extern "system" fn() -> isize, FnWindowsGetStringRawBuffer>(f))
    })
}

/// Try reading as HSTRING (ptr is an HSTRING handle).
unsafe fn hstring_to_string(hstr: *mut c_void) -> Option<String> {
    let wgsrb = get_wgsrb()?;
    let mut len: u32 = 0;
    let raw = wgsrb(hstr, &mut len);
    if raw.is_null() || len == 0 {
        return None;
    }
    String::from_utf16(std::slice::from_raw_parts(raw, len as usize)).ok()
}

/// Create an HSTRING from a Rust &str and write it to the out-param.
unsafe fn create_hstring(text: &str, out: *mut *mut c_void) -> Hresult {
    let wcs = match get_wcs() {
        Some(f) => f,
        None => return -1,
    };
    let wide: Vec<u16> = text.encode_utf16().collect();
    let mut hstr: *mut c_void = std::ptr::null_mut();
    let hr = wcs(wide.as_ptr(), wide.len() as u32, &mut hstr);
    if hr >= 0 && !out.is_null() {
        *out = hstr;
    }
    hr
}

/// The detour for Bcp47GetAbbreviation.
///
/// Signature: `Hresult Bcp47GetAbbreviation(HSTRING bcp47Tag, HSTRING* result)`
///
/// Strategy: call original first. If it returns ERROR_NOT_FOUND (0x80070490),
/// look up in our config. If we have an abbreviation, write it.
unsafe extern "system" fn hook_bcp47_get_abbreviation(
    bcp47_tag: *mut c_void,
    result: *mut *mut c_void,
) -> Hresult {
    let orig: FnBcp47GetAbbreviation = std::mem::transmute(ORIG_BCP47_GET_ABBREVIATION);
    let hr = orig(bcp47_tag, result);

    debug_log(&format!("  original returned: 0x{hr:08X}"));

    // 0x80070490 = Hresult_FROM_WIN32(ERROR_NOT_FOUND)
    if hr == 0x80070490u32 as i32 && !result.is_null() {
        let tag_str = hstring_to_string(bcp47_tag).unwrap_or_default();
        debug_log(&format!("  ERROR_NOT_FOUND for tag={tag_str}"));
        let tag_lower = tag_str.to_lowercase();
        if let Some(config) = CONFIG.get() {
            debug_log(&format!(
                "  config has {} entries, keys: {:?}",
                config.len(),
                config.keys().collect::<Vec<_>>()
            ));
            if let Some(entry) = config.get(&tag_lower).or_else(|| config.get(&tag_str)) {
                debug_log(&format!("  matched! abbr={:?}", entry.abbreviation));
                if let Some(abbr) = entry
                    .abbreviation
                    .as_ref()
                    .and_then(crate::ui_language::resolve)
                {
                    let create_hr = create_hstring(abbr, result);
                    debug_log(&format!("  create_hstring returned: 0x{create_hr:08X}"));
                    if create_hr >= 0 {
                        return S_OK;
                    }
                }
            } else {
                debug_log(&format!("  no match for tag={tag_str} / lower={tag_lower}"));
            }
        } else {
            debug_log("  CONFIG not set!");
        }
    }

    hr
}

// --- get_DisplayName hook ---
// ILanguage::get_DisplayName is vtable slot 7 (IUnknown 3 + IInspectable 3 + get_LanguageTag + get_DisplayName)
// Signature: Hresult get_DisplayName(ILanguage* this, HSTRING* value)
type FnGetDisplayName = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult;
static mut ORIG_GET_DISPLAY_NAME: *mut c_void = std::ptr::null_mut();

/// To match the display name override, we need the Language object's LanguageTag.
/// get_LanguageTag is vtable slot 6.
unsafe fn get_language_tag_from_this(this: *mut c_void) -> Option<String> {
    let vtable = *(this as *const *const usize);
    let get_tag: FnGetDisplayName = std::mem::transmute(*vtable.add(6));
    let mut hstr: *mut c_void = std::ptr::null_mut();
    let hr = get_tag(this, &mut hstr);
    if hr < 0 || hstr.is_null() {
        return None;
    }
    let result = hstring_to_string(hstr);
    // Delete the HSTRING
    if let Some(wds) = get_wds() {
        wds(hstr);
    }
    result
}

type FnWindowsDeleteString = unsafe extern "system" fn(*mut c_void) -> Hresult;
static WDS: OnceLock<Option<FnWindowsDeleteString>> = OnceLock::new();

fn get_wds() -> Option<FnWindowsDeleteString> {
    *WDS.get_or_init(|| unsafe {
        let name: Vec<u16> = "combase.dll\0".encode_utf16().collect();
        let module = windows_sys::Win32::System::LibraryLoader::LoadLibraryW(name.as_ptr());
        if module.is_null() {
            return None;
        }
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            module,
            c"WindowsDeleteString".as_ptr().cast(),
        )
        .map(|f| {
            std::mem::transmute::<unsafe extern "system" fn() -> isize, FnWindowsDeleteString>(f)
        })
    })
}

unsafe extern "system" fn hook_get_display_name(
    this: *mut c_void,
    result: *mut *mut c_void,
) -> Hresult {
    // Call original first
    let orig: FnGetDisplayName = std::mem::transmute(ORIG_GET_DISPLAY_NAME);
    let hr = orig(this, result);

    // Check config for display_name override
    if let Some(config) = CONFIG.get() {
        if let Some(tag) = get_language_tag_from_this(this) {
            let tag_lower = tag.to_lowercase();
            if let Some(entry) = config.get(&tag_lower).or_else(|| config.get(&tag)) {
                if let Some(display_name) = entry
                    .display_name
                    .as_ref()
                    .and_then(crate::ui_language::resolve)
                {
                    debug_log(&format!(
                        "get_DisplayName override: tag={tag} -> {display_name}"
                    ));
                    // Preserve the original result if allocation fails.
                    let mut replacement = std::ptr::null_mut();
                    let create_hr = create_hstring(display_name, &mut replacement);
                    if create_hr >= 0 && !result.is_null() {
                        if hr >= 0 && !(*result).is_null() {
                            if let Some(wds) = get_wds() {
                                wds(*result);
                            }
                        }
                        *result = replacement;
                        return S_OK;
                    } else if !replacement.is_null() {
                        if let Some(wds) = get_wds() {
                            wds(replacement);
                        }
                    }
                }
            }
        }
    }

    hr
}

static CONFIG: OnceLock<LocaleConfig> = OnceLock::new();

/// Resolve get_DisplayName vtable address by creating a Language("en-US") object.
unsafe fn resolve_get_display_name() -> Result<*mut c_void, String> {
    type FnRoInitialize = unsafe extern "system" fn(u32) -> Hresult;
    type FnRoGetActivationFactory =
        unsafe extern "system" fn(*mut c_void, *const [u8; 16], *mut *mut c_void) -> Hresult;

    let wide_combase: Vec<u16> = "combase.dll\0".encode_utf16().collect();
    let combase = windows_sys::Win32::System::LibraryLoader::LoadLibraryW(wide_combase.as_ptr());
    if combase.is_null() {
        return Err("combase.dll not found".to_string());
    }

    let get_fn = |name: &[u8]| -> Option<*mut c_void> {
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(combase, name.as_ptr())
            .map(|f| f as *mut c_void)
    };

    let ro_init: FnRoInitialize =
        std::mem::transmute(get_fn(b"RoInitialize\0").ok_or("RoInitialize not found")?);
    let ro_get_af: FnRoGetActivationFactory = std::mem::transmute(
        get_fn(b"RoGetActivationFactory\0").ok_or("RoGetActivationFactory not found")?,
    );
    let wcs: FnWindowsCreateString = std::mem::transmute(
        get_fn(b"WindowsCreateString\0").ok_or("WindowsCreateString not found")?,
    );
    let wds: FnWindowsDeleteString = std::mem::transmute(
        get_fn(b"WindowsDeleteString\0").ok_or("WindowsDeleteString not found")?,
    );

    // Initialize WinRT
    let hr = ro_init(1);
    if hr < 0 && hr != -2147417850 {
        // RPC_E_CHANGED_MODE
        return Err(format!("RoInitialize: 0x{:08X}", hr as u32));
    }

    // ILanguageFactory IID: {9B0252AC-0C27-44F8-B792-9793FB66C63E}
    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }
    let factory_iid = Guid {
        data1: 0x9B0252AC,
        data2: 0x0C27,
        data3: 0x44F8,
        data4: [0xB7, 0x92, 0x97, 0x93, 0xFB, 0x66, 0xC6, 0x3E],
    };

    // Create class name HSTRING
    let class_name = "Windows.Globalization.Language";
    let class_wide: Vec<u16> = class_name.encode_utf16().collect();
    let mut class_hstr: *mut c_void = std::ptr::null_mut();
    let hr = wcs(
        class_wide.as_ptr(),
        class_wide.len() as u32,
        &mut class_hstr,
    );
    if hr < 0 {
        return Err(format!("WindowsCreateString(class): 0x{:08X}", hr as u32));
    }

    // Get activation factory
    let mut factory: *mut c_void = std::ptr::null_mut();
    let hr = ro_get_af(
        class_hstr,
        &factory_iid as *const Guid as *const [u8; 16],
        &mut factory,
    );
    wds(class_hstr);
    if hr < 0 {
        return Err(format!("RoGetActivationFactory: 0x{:08X}", hr as u32));
    }

    // Call ILanguageFactory::CreateLanguage (slot 6): Hresult(this, HSTRING tag, ILanguage** out)
    let tag_wide: Vec<u16> = "en-US".encode_utf16().collect();
    let mut tag_hstr: *mut c_void = std::ptr::null_mut();
    let hr = wcs(tag_wide.as_ptr(), tag_wide.len() as u32, &mut tag_hstr);
    if hr < 0 {
        return Err(format!("WindowsCreateString(tag): 0x{:08X}", hr as u32));
    }

    let factory_vtable = *(factory as *const *const usize);
    let create_fn: extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> Hresult =
        std::mem::transmute(*factory_vtable.add(6));

    let mut instance: *mut c_void = std::ptr::null_mut();
    let hr = create_fn(factory, tag_hstr, &mut instance);
    wds(tag_hstr);

    // Release factory
    let release: extern "system" fn(*mut c_void) -> u32 =
        std::mem::transmute(*factory_vtable.add(2));
    release(factory);

    if hr < 0 {
        return Err(format!("CreateLanguage: 0x{:08X}", hr as u32));
    }

    // Read vtable slot 7 (get_DisplayName)
    let vtable = *(instance as *const *const usize);
    let fn_ptr = *vtable.add(7);

    debug_log(&format!("get_DisplayName resolved: fn_ptr=0x{fn_ptr:X}"));

    // Release instance
    let release2: extern "system" fn(*mut c_void) -> u32 = std::mem::transmute(*vtable.add(2));
    release2(instance);

    Ok(fn_ptr as *mut c_void)
}

pub fn install_hooks(config: LocaleConfig) -> Result<(), String> {
    debug_log(&format!("install_hooks: {} entries", config.len()));
    for (k, v) in &config {
        debug_log(&format!("  key={k:?} abbr={:?}", v.abbreviation));
    }
    CONFIG
        .set(config)
        .map_err(|_| "config already set".to_string())?;

    get_wcs().ok_or("WindowsCreateString unavailable")?;
    get_wgsrb().ok_or("WindowsGetStringRawBuffer unavailable")?;
    get_wds().ok_or("WindowsDeleteString unavailable")?;

    // Resolve Bcp47GetAbbreviation from bcp47langs.dll
    let target = unsafe {
        let wide: Vec<u16> = "bcp47langs.dll\0".encode_utf16().collect();
        let hmod = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(wide.as_ptr());
        let hmod = if hmod.is_null() {
            windows_sys::Win32::System::LibraryLoader::LoadLibraryW(wide.as_ptr())
        } else {
            hmod
        };
        if hmod.is_null() {
            return Err("bcp47langs.dll not found".to_string());
        }
        windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            hmod,
            c"Bcp47GetAbbreviation".as_ptr().cast(),
        )
    };
    let target_addr = match target {
        Some(f) => f as *mut c_void,
        None => return Err("Bcp47GetAbbreviation not found".to_string()),
    };

    // Resolve get_DisplayName vtable address
    let display_name_addr = unsafe { resolve_get_display_name() }?;

    unsafe {
        ORIG_BCP47_GET_ABBREVIATION = target_addr;

        let mut txn = windows_detours::Transaction::new()
            .map_err(|e| format!("DetourTransactionBegin: {e}"))?;
        txn.update_current_thread()
            .map_err(|e| format!("DetourUpdateThread: {e}"))?;

        // Hook Bcp47GetAbbreviation
        let hook1 = windows_detours::RawDetourHook::new(
            &raw mut ORIG_BCP47_GET_ABBREVIATION,
            hook_bcp47_get_abbreviation as *mut c_void,
        );
        txn.attach_raw(&hook1)
            .map_err(|e| format!("DetourAttach(Bcp47): {e}"))?;

        // Hook get_DisplayName if resolved
        {
            ORIG_GET_DISPLAY_NAME = display_name_addr;
            let hook2 = windows_detours::RawDetourHook::new(
                &raw mut ORIG_GET_DISPLAY_NAME,
                hook_get_display_name as *mut c_void,
            );
            txn.attach_raw(&hook2)
                .map_err(|e| format!("DetourAttach(DisplayName): {e}"))?;
        }

        txn.commit()
            .map_err(|e| format!("DetourTransactionCommit: {e}"))?;
    }

    Ok(())
}

#[cfg(test)]
pub fn uninstall_hooks() {
    unsafe {
        let has_bcp47 = !ORIG_BCP47_GET_ABBREVIATION.is_null();
        let has_display = !ORIG_GET_DISPLAY_NAME.is_null();
        if !has_bcp47 && !has_display {
            return;
        }
        if let Ok(mut txn) = windows_detours::Transaction::new() {
            let _ = txn.update_current_thread();
            if has_bcp47 {
                let hook = windows_detours::RawDetourHook::new(
                    &raw mut ORIG_BCP47_GET_ABBREVIATION,
                    hook_bcp47_get_abbreviation as *mut c_void,
                );
                let _ = txn.detach_raw(&hook);
            }
            if has_display {
                let hook = windows_detours::RawDetourHook::new(
                    &raw mut ORIG_GET_DISPLAY_NAME,
                    hook_get_display_name as *mut c_void,
                );
                let _ = txn.detach_raw(&hook);
            }
            let _ = txn.commit();
        }
        ORIG_BCP47_GET_ABBREVIATION = std::ptr::null_mut();
        ORIG_GET_DISPLAY_NAME = std::ptr::null_mut();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::{core::HSTRING, Globalization::Language};
    use windows_sys::Win32::Globalization::{SetThreadPreferredUILanguages, MUI_LANGUAGE_NAME};

    #[test]
    fn real_hooks_localize_using_the_calling_threads_ui_language() {
        if !wind_common::os::eligible() {
            return;
        }
        let _transaction_lock = crate::DETOUR_TEST_LOCK.lock().unwrap();
        // UI preferences are confined to this disposable thread. The user's
        // Windows language settings and Explorer are never changed by this test.
        std::thread::spawn(|| unsafe {
            fn set_ui_language(tag: &str) {
                let wide: Vec<u16> = tag.encode_utf16().chain([0, 0]).collect();
                let mut count = 0;
                assert_ne!(
                    unsafe {
                        SetThreadPreferredUILanguages(MUI_LANGUAGE_NAME, wide.as_ptr(), &mut count)
                    },
                    0
                );
                assert_eq!(count, 1);
            }
            fn display_name(tag: &str) -> String {
                Language::CreateLanguage(&HSTRING::from(tag))
                    .unwrap()
                    .DisplayName()
                    .unwrap()
                    .to_string_lossy()
            }

            // Resolving the method also initializes WinRT on this thread.
            resolve_get_display_name().unwrap();
            set_ui_language("en-US");
            let german_before = display_name("de-DE");
            let config: LocaleConfig =
                serde_json::from_str(include_str!("../../../tests/fixtures/localized.json"))
                    .unwrap();
            install_hooks(config).unwrap();
            struct Hooks;
            impl Drop for Hooks {
                fn drop(&mut self) {
                    uninstall_hooks();
                }
            }
            let _hooks = Hooks;
            let module: Vec<u16> = "bcp47langs.dll\0".encode_utf16().collect();
            let module =
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(module.as_ptr());
            let abbreviation: FnBcp47GetAbbreviation = std::mem::transmute(
                windows_sys::Win32::System::LibraryLoader::GetProcAddress(
                    module,
                    c"Bcp47GetAbbreviation".as_ptr().cast(),
                )
                .unwrap(),
            );

            for (ui, kildin, futhark) in [
                ("en-US", "Kildin Sami", "Futhark"),
                ("ru-RU", "Кильдинский саамский", "Футарк"),
                ("ru-BY", "Кильдинский саамский", "Футарк"),
                ("en-GB", "Kildin Sami", "Futhark"),
                ("sv-SE", "Kildin Sami", "Futhark"),
            ] {
                set_ui_language(ui);
                assert_eq!(display_name("sjd-Cyrl"), kildin, "UI language {ui}");
                assert_eq!(display_name("gem-Runr"), futhark, "UI language {ui}");
                for (tag, expected) in [("sjd-Cyrl", "КИЛ"), ("gem-Runr", "RUN")] {
                    let mut tag_hstring = std::ptr::null_mut();
                    assert_eq!(create_hstring(tag, &mut tag_hstring), S_OK);
                    let mut value = std::ptr::null_mut();
                    let hr = abbreviation(tag_hstring, &mut value);
                    get_wds().unwrap()(tag_hstring);
                    assert_eq!(hr, S_OK, "abbreviation for {tag} in {ui}");
                    let actual = hstring_to_string(value).unwrap();
                    get_wds().unwrap()(value);
                    assert_eq!(actual, expected, "abbreviation for {tag} in {ui}");
                }
            }
            set_ui_language("en-US");
            assert_eq!(display_name("de-DE"), german_before);
        })
        .join()
        .unwrap();
    }
}
