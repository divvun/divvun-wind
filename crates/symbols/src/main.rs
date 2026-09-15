use std::path::PathBuf;
use wind_symbols::{image, resolver, MODULES};

fn main() {
    if !wind_common::os::eligible() {
        return;
    }
    if let Err(e) = run() {
        eprintln!("taskbar symbols: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "--help".into());
    if command == "--help" || command == "-h" {
        println!("divvun-wind-symbols <prepare|verify> [--pid PID | --image DLL]\n  [--cache-dir DIR] [--symbol-path PATH] [--dbghelp DLL] [--offline] [--refresh]\n\nDefaults to the current shell's loaded SystemTray.dll / Taskbar.View.dll.\nprepare downloads matching Microsoft symbols when needed; --offline forbids network symbol paths.\nverify only validates the existing cache. --refresh forces fresh symbol lookup.");
        return Ok(());
    }
    if command != "prepare" && command != "verify" {
        return Err(format!("unknown command {command}"));
    }
    let mut pid = None;
    let mut image_path = None;
    let mut cache_dir = wind_symbols::default_cache_dir();
    let mut symbol_path = None;
    let mut dbghelp = resolver::default_dbghelp();
    let mut offline = false;
    let mut refresh = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--offline" => offline = true,
            "--refresh" => refresh = true,
            "--pid" => {
                pid = Some(
                    args.next()
                        .ok_or("--pid needs a value")?
                        .parse::<u32>()
                        .map_err(|_| "invalid pid")?,
                )
            }
            "--image" => {
                image_path = Some(PathBuf::from(args.next().ok_or("--image needs a value")?))
            }
            "--cache-dir" => {
                cache_dir = PathBuf::from(args.next().ok_or("--cache-dir needs a value")?)
            }
            "--symbol-path" => {
                symbol_path = Some(args.next().ok_or("--symbol-path needs a value")?)
            }
            "--dbghelp" => dbghelp = PathBuf::from(args.next().ok_or("--dbghelp needs a value")?),
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    if pid.is_some() && image_path.is_some() {
        return Err("use either --pid or --image".into());
    }
    // SymSrv requires an absolute downstream cache path. Do not canonicalize:
    // a first-time preparation has not created this directory yet.
    let cache_dir = std::path::absolute(cache_dir).map_err(|e| e.to_string())?;
    let pdb_cache = cache_dir.join("pdb");
    let default_symbol_path = symbol_path.is_none();
    let symbol_path = symbol_path.unwrap_or_else(|| {
        if offline {
            pdb_cache.to_string_lossy().into_owned()
        } else {
            format!(
                "srv*{}*https://msdl.microsoft.com/download/symbols",
                pdb_cache.display()
            )
        }
    });
    let images = if let Some(path) = image_path {
        vec![path]
    } else {
        process_images(pid)?
    };
    let mut errors = Vec::new();
    for path in images {
        let attempt = (|| {
            let path = path.canonicalize().map_err(|e| e.to_string())?;
            let module = path
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or("invalid module name")?;
            if !MODULES.iter().any(|n| n.eq_ignore_ascii_case(module)) {
                return Err("expected SystemTray.dll or Taskbar.View.dll".into());
            }
            let (headers, identity) = image::inspect_file(&path)?;
            let existing = wind_symbols::read_cache(&cache_dir, module, &headers, &identity);
            let (cache, reused) = if command == "verify" || (!refresh && existing.is_ok()) {
                (existing?, true)
            } else {
                if offline {
                    validate_offline_path(&symbol_path)?;
                } else if default_symbol_path {
                    std::fs::create_dir_all(&pdb_cache).map_err(|e| e.to_string())?;
                }
                let cache =
                    resolver::resolve(&path, module, &headers, &identity, &symbol_path, &dbghelp)?;
                wind_symbols::write_cache(&cache_dir, &cache)?;
                (cache, false)
            };
            println!(
                "{}",
                serde_json::json!({ "image": path, "cache": wind_symbols::cache_path(&cache_dir, module, &identity),
                "reused": reused, "module": cache.module, "identity": cache.identity,
                "build_rva": format!("0x{:X}", cache.build.rva), "update_rva": format!("0x{:X}", cache.update.rva) })
            );
            Ok::<(), String>(())
        })();
        match attempt {
            Ok(()) => return Ok(()),
            Err(e) => errors.push(format!("{}: {e}", path.display())),
        }
    }
    Err(format!(
        "no compatible symbol cache could be {}:\n{}",
        if command == "verify" {
            "verified"
        } else {
            "prepared"
        },
        errors.join("\n")
    ))
}

fn validate_offline_path(symbol_path: &str) -> Result<(), String> {
    use std::path::{Component, Prefix};
    use windows_sys::Win32::{
        Storage::FileSystem::GetDriveTypeW, System::WindowsProgramming::DRIVE_REMOTE,
    };
    for part in symbol_path.split(';') {
        if part.contains('*') || part.contains("://") {
            return Err("--offline accepts local symbol directories only".into());
        }
        let path = PathBuf::from(part)
            .canonicalize()
            .map_err(|e| format!("offline symbol directory {part}: {e}"))?;
        let drive = match path.components().next() {
            Some(Component::Prefix(p)) => match p.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => d,
                _ => return Err("--offline does not accept network directories".into()),
            },
            _ => return Err("offline symbol directory must be on a local drive".into()),
        };
        let root = [drive as u16, b':' as u16, b'\\' as u16, 0];
        if unsafe { GetDriveTypeW(root.as_ptr()) } == DRIVE_REMOTE {
            return Err("--offline does not accept mapped network drives".into());
        }
    }
    Ok(())
}

fn process_images(pid: Option<u32>) -> Result<Vec<PathBuf>, String> {
    use windows_sys::Win32::{
        Foundation::*, System::Diagnostics::ToolHelp::*, UI::WindowsAndMessaging::*,
    };
    let pid = match pid {
        Some(pid) => pid,
        None => {
            let mut pid = 0;
            unsafe {
                GetWindowThreadProcessId(GetShellWindow(), &mut pid);
            }
            if pid == 0 {
                return Err("no shell in this session; specify --pid or --image".into());
            }
            pid
        }
    };
    unsafe {
        // Explorer loads many DLLs concurrently at startup. Windows explicitly
        // requires retrying ERROR_BAD_LENGTH while the module list changes.
        let mut snapshot = INVALID_HANDLE_VALUE;
        for _ in 0..10 {
            snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid);
            if snapshot != INVALID_HANDLE_VALUE || GetLastError() != ERROR_BAD_LENGTH {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(format!(
                "snapshot pid {pid}: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut entry: MODULEENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of_val(&entry) as u32;
        let mut paths = Vec::new();
        let mut ok = Module32FirstW(snapshot, &mut entry);
        while ok != 0 {
            let name = String::from_utf16_lossy(
                &entry.szModule[..entry
                    .szModule
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szModule.len())],
            );
            if MODULES.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                paths.push(PathBuf::from(String::from_utf16_lossy(
                    &entry.szExePath[..entry
                        .szExePath
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExePath.len())],
                )));
            }
            ok = Module32NextW(snapshot, &mut entry);
        }
        let enumeration_error = GetLastError();
        CloseHandle(snapshot);
        if enumeration_error != ERROR_NO_MORE_FILES {
            return Err(format!(
                "enumerate modules in pid {pid}: {}",
                std::io::Error::from_raw_os_error(enumeration_error as i32)
            ));
        }
        paths.sort_by_key(|p| {
            MODULES
                .iter()
                .position(|n| {
                    p.file_name()
                        .unwrap()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(n)
                })
                .unwrap_or(2)
        });
        if paths.is_empty() {
            return Err(format!("neither shell module is loaded in pid {pid}"));
        }
        Ok(paths)
    }
}
