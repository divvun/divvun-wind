#![cfg_attr(not(test), windows_subsystem = "windows")]
mod native;

use native::{Handle, Process};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::{Foundation::*, Storage::FileSystem::SYNCHRONIZE, System::Threading::*};

const MUTEX: &str = "Local\\Divvun.Wind";
const STOP: &str = "Local\\Divvun.Wind.Stop";

fn root() -> Result<PathBuf, String> {
    Ok(
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA is unset")?)
            .join("Divvun")
            .join("Wind"),
    )
}
fn state_path(session: u32) -> Result<PathBuf, String> {
    Ok(root()?.join(format!("wind-{session}.json")))
}

#[derive(Serialize, Deserialize)]
struct Status {
    watcher_pid: u32,
    watcher_created: u64,
    session: u32,
    explorer_pid: Option<u32>,
    explorer_created: Option<u64>,
    state: String,
    detail: String,
    unix_ms: u128,
}

struct Reporter {
    status: Status,
    log: PathBuf,
}
impl Reporter {
    fn new(session: u32) -> Result<Self, String> {
        fs::create_dir_all(root()?).map_err(|e| e.to_string())?;
        let log = root()?.join(format!("wind-{session}.log"));
        Ok(Self {
            log,
            status: Status {
                watcher_pid: std::process::id(),
                watcher_created: native::creation_time(unsafe { GetCurrentProcess() })?,
                session,
                explorer_pid: None,
                explorer_created: None,
                state: String::new(),
                detail: String::new(),
                unix_ms: 0,
            },
        })
    }
    fn set(
        &mut self,
        state: &str,
        process: Option<&Process>,
        detail: impl Into<String>,
    ) -> Result<(), String> {
        let detail = detail.into();
        let pid = process.map(|p| p.pid);
        let created = process.map(|p| p.created);
        if self.status.state == state
            && self.status.detail == detail
            && self.status.explorer_pid == pid
            && self.status.explorer_created == created
        {
            return Ok(());
        }
        self.status.state = state.into();
        self.status.detail = detail;
        self.status.explorer_pid = pid;
        self.status.explorer_created = created;
        self.status.unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let bytes = serde_json::to_vec_pretty(&self.status).map_err(|e| e.to_string())?;
        let path = state_path(self.status.session)?;
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temp, bytes)
            .and_then(|_| fs::rename(&temp, &path))
            .map_err(|e| e.to_string())?;
        if fs::metadata(&self.log).is_ok_and(|m| m.len() > 1024 * 1024) {
            let _ = fs::remove_file(self.log.with_extension("previous.log"));
            fs::rename(&self.log, self.log.with_extension("previous.log"))
                .map_err(|e| e.to_string())?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .map_err(|e| e.to_string())?;
        writeln!(
            file,
            "{} {} explorer={pid:?} {}",
            self.status.unix_ms, state, self.status.detail
        )
        .map_err(|e| e.to_string())
    }
}

struct Instance {
    mutex: Handle,
    stop: Handle,
}
impl Instance {
    fn acquire() -> Result<Option<Self>, String> {
        unsafe {
            let mutex = Handle::checked(
                CreateMutexW(std::ptr::null(), 0, native::wide(MUTEX).as_ptr()),
                "watcher mutex",
            )?;
            match WaitForSingleObject(mutex.0, 0) {
                WAIT_TIMEOUT => return Ok(None),
                WAIT_OBJECT_0 | WAIT_ABANDONED_0 => {}
                _ => return Err(native::error("acquire watcher mutex")),
            }
            let event = CreateEventW(std::ptr::null(), 1, 0, native::wide(STOP).as_ptr());
            if event.is_null() {
                ReleaseMutex(mutex.0);
                return Err(native::error("stop event"));
            }
            let stop = Handle(event);
            ResetEvent(stop.0);
            Ok(Some(Self { mutex, stop }))
        }
    }
}
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.mutex.0);
        }
    }
}

fn prepare(process: &Process, assets: &Path, stop: &Handle, log: &Path) -> Result<(), String> {
    let output = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| e.to_string())?;
    let mut child = Command::new(assets.join("divvun-wind-symbols.exe"))
        .args(["prepare", "--pid", &process.pid.to_string(), "--cache-dir"])
        .arg(wind_symbols::default_cache_dir())
        .stdin(Stdio::null())
        .stdout(Stdio::from(output.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(output))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("start symbol resolver: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "symbol preparation failed ({status}); see watcher log"
                ))
            }
            Ok(None) => {}
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
        let wake = native::wait(stop, Some(process), 100);
        if Instant::now() >= deadline || !matches!(wake, Ok(false)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(
                "symbol preparation stopped, target exited, or 120-second deadline expired".into(),
            );
        }
    }
}

fn validate_assets(assets: &Path) -> Result<PathBuf, String> {
    let dll = assets.join("divvunwind.dll");
    for path in [&dll, &assets.join("divvun-wind-symbols.exe")] {
        if !path.is_file() {
            return Err(format!(
                "missing {}; install the standalone bundle first",
                path.display()
            ));
        }
    }
    Ok(dll)
}

fn run() -> Result<(), String> {
    let Some(instance) = Instance::acquire()? else {
        return Ok(());
    };
    let session = native::session_id()?;
    let mut reporter = Reporter::new(session)?;
    let assets = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("executable has no directory")?
        .to_path_buf();
    let dll = match validate_assets(&assets) {
        Ok(dll) => dll,
        Err(e) => {
            reporter.set("error", None, &e)?;
            return Err(e);
        }
    };
    reporter.set("starting", None, format!("bundle={}", assets.display()))?;
    let result = monitor(&instance.stop, &dll, &assets, &mut reporter);
    if let Err(e) = &result {
        reporter.set("error", None, e)?;
    } else {
        reporter.set(
            "stopped",
            None,
            "watcher stopped; installed hooks remain in Explorer",
        )?;
    }
    result
}

fn retry_delay(failures: u32) -> u32 {
    (1000u32.saturating_mul(1u32 << failures.min(6))).min(60000)
}

fn monitor(
    stop: &Handle,
    dll: &Path,
    assets: &Path,
    reporter: &mut Reporter,
) -> Result<(), String> {
    let directory = wind_common::locale::catalogue_directory()?;
    let mut catalogue = wind_common::catalogue::Loader::default();
    let mut target: Option<Process> = None;
    let mut attempted = false;
    let mut failures = 0;
    let mut loaded_since: Option<Instant> = None;
    let mut discovered_at = Instant::now();
    loop {
        if stop.signaled() {
            return Ok(());
        }
        if target.as_ref().is_some_and(|p| p.handle.signaled()) {
            reporter.set(
                "explorer_exited",
                target.as_ref(),
                "waiting for replacement taskbar",
            )?;
            target = None;
            attempted = false;
            failures = 0;
            loaded_since = None;
        }
        if target.is_none() {
            match native::shell(reporter.status.session) {
                Ok(Some(process)) => {
                    discovered_at = Instant::now();
                    reporter.set(
                        "explorer_found",
                        Some(&process),
                        "waiting for taskbar modules",
                    )?;
                    target = Some(process);
                }
                result => {
                    reporter.set(
                        "waiting_for_explorer",
                        None,
                        match result {
                            Err(e) => e,
                            _ => "no taskbar in this session".into(),
                        },
                    )?;
                    native::wait(stop, None, 500)?;
                    continue;
                }
            }
        }
        let process = target.as_ref().unwrap();
        let modules = match native::modules(process.pid) {
            Ok(modules) => modules,
            Err(e) => {
                reporter.set("waiting_for_modules", Some(process), e)?;
                native::wait(stop, Some(process), 1000)?;
                continue;
            }
        };
        if let Some(module) = modules
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case("divvunwind.dll"))
        {
            if process.ready() {
                let state = if native::same_path(&module.path, dll) {
                    "active"
                } else {
                    "active_previous_bundle"
                };
                reporter.set(
                    state,
                    Some(process),
                    format!(
                        "initialization acknowledged; UI refresh not confirmed; DLL={}",
                        module.path.display()
                    ),
                )?;
                native::wait(stop, Some(process), 30000)?;
            } else {
                let since = loaded_since.get_or_insert_with(Instant::now);
                let state = if since.elapsed() < Duration::from_secs(15) {
                    "initializing"
                } else {
                    "loaded_without_readiness"
                };
                reporter.set(
                    state,
                    Some(process),
                    format!(
                        "DLL={}; no readiness signal yet; no duplicate injection",
                        module.path.display()
                    ),
                )?;
                native::wait(
                    stop,
                    Some(process),
                    if state == "initializing" { 250 } else { 5000 },
                )?;
            }
            continue;
        }
        if attempted {
            reporter.set("load_unconfirmed", Some(process), "one load attempt was made; waiting for process exit to avoid duplicate remote threads")?;
            native::wait(stop, Some(process), 5000)?;
            continue;
        }
        let snapshot = catalogue.scan(&directory);
        if snapshot.entries.is_empty() {
            reporter.set(
                "idle",
                Some(process),
                format!(
                    "no usable keyboard labels; {} diagnostic(s)",
                    snapshot.diagnostics.len()
                ),
            )?;
            native::wait(stop, Some(process), 1000)?;
            continue;
        }
        if !modules.iter().any(|m| {
            wind_symbols::MODULES
                .iter()
                .any(|n| m.name.eq_ignore_ascii_case(n))
        }) {
            reporter.set(
                "waiting_for_modules",
                Some(process),
                "taskbar implementation not loaded yet",
            )?;
            native::wait(stop, Some(process), 250)?;
            continue;
        }
        // On split-shell builds Taskbar.View arrives before SystemTray. Starting
        // PDB lookup in that gap needlessly resolves the old container. Give the
        // sibling DLL time to load, then permit the legacy container as a fallback.
        let split_image = modules
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case("Taskbar.View.dll"))
            .is_some_and(|m| m.path.with_file_name("SystemTray.dll").is_file());
        if split_image
            && !modules
                .iter()
                .any(|m| m.name.eq_ignore_ascii_case("SystemTray.dll"))
            && discovered_at.elapsed() < Duration::from_secs(5)
        {
            reporter.set(
                "waiting_for_modules",
                Some(process),
                "waiting for the separate SystemTray.dll implementation",
            )?;
            native::wait(stop, Some(process), 100)?;
            continue;
        }
        reporter.set(
            "preparing_symbols",
            Some(process),
            "checking cache; downloading matching Microsoft symbols only if needed",
        )?;
        if let Err(e) = prepare(process, assets, stop, &reporter.log) {
            failures += 1;
            reporter.set("retrying_symbols", Some(process), e)?;
            native::wait(stop, Some(process), retry_delay(failures))?;
            continue;
        }
        if process.handle.signaled() || stop.signaled() {
            continue;
        }
        // Holding the process handle binds injection to this exact process, even if
        // it exits and its PID is reused. A failed load is never blindly repeated.
        attempted = true;
        reporter.set("injecting", Some(process), dll.display().to_string())?;
        if let Err(e) = native::inject(process, dll) {
            reporter.set("load_error", Some(process), e)?;
        }
        loaded_since = Some(Instant::now());
        native::wait(stop, Some(process), 100)?;
    }
}

fn stop() -> Result<(), String> {
    unsafe {
        let raw = OpenEventW(EVENT_MODIFY_STATE, 0, native::wide(STOP).as_ptr());
        if raw.is_null() {
            if GetLastError() == ERROR_FILE_NOT_FOUND {
                println!("{{\"stopped\":true}}");
                return Ok(());
            }
            return Err(native::error("open stop event"));
        }
        let event = Handle(raw);
        if SetEvent(event.0) == 0 {
            return Err(native::error("signal stop"));
        }
        let raw_mutex = OpenMutexW(
            SYNCHRONIZE | MUTEX_MODIFY_STATE,
            0,
            native::wide(MUTEX).as_ptr(),
        );
        if !raw_mutex.is_null() {
            let mutex = Handle(raw_mutex);
            match WaitForSingleObject(mutex.0, 15000) {
                WAIT_OBJECT_0 | WAIT_ABANDONED_0 => {
                    ReleaseMutex(mutex.0);
                }
                _ => return Err("watcher did not stop within 15 seconds".into()),
            }
        }
    }
    println!("{{\"stopped\":true}}");
    Ok(())
}

fn status() -> Result<(), String> {
    let path = state_path(native::session_id()?)?;
    if !path.is_file() {
        println!("{{\"watcher_running\":false,\"state\":\"not_started\"}}");
        return Ok(());
    }
    let status: Status = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let running = Process::open(status.watcher_pid, false)
        .is_ok_and(|p| p.created == status.watcher_created && !p.handle.signaled());
    let mut value = serde_json::to_value(status).map_err(|e| e.to_string())?;
    value["watcher_running"] = running.into();
    println!(
        "{}",
        serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn check(snapshot: wind_common::catalogue::Snapshot) -> Result<(), String> {
    let valid = snapshot.valid();
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({"valid": valid, "catalogue": snapshot}))
            .map_err(|e| e.to_string())?
    );
    if valid {
        Ok(())
    } else {
        Err("catalogue validation failed".into())
    }
}

fn main() {
    // This guard precedes argument parsing, events, log files and shell access.
    if !wind_common::os::eligible() {
        return;
    }
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = match args.as_slice() {
        [] => run(),
        [command] if command == "run" => run(),
        [command] if command == "stop" => stop(),
        [command] if command == "status" => status(),
        [command] if command == "check" => wind_common::locale::catalogue_directory()
            .and_then(|p| check(wind_common::catalogue::Loader::default().scan(&p))),
        [command, option, directory] if command == "check" && option == "--directory" => {
            check(wind_common::catalogue::Loader::default().scan(Path::new(directory)))
        }
        [command, file] if command == "check-file" => {
            check(wind_common::catalogue::check_file(Path::new(file)))
        }
        [command] if command == "diagnose-os" => {
            println!(
                "{}",
                serde_json::json!({"eligible": true, "version": wind_common::os::current()})
            );
            Ok(())
        }
        [command] if command == "--help" || command == "-h" => {
            println!("divvun-wind [run|status|stop|check [--directory DIR]|check-file FILE|diagnose-os]\nPer-session Explorer watcher. stop leaves the DLL active until Explorer exits.");
            Ok(())
        }
        _ => Err("invalid arguments; use divvun-wind --help".into()),
    };
    if let Err(e) = result {
        eprintln!("divvun-wind: {e}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_exit_wakes_monitor_and_new_process_has_distinct_identity() {
        let stop = Handle::checked(
            unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) },
            "test event",
        )
        .unwrap();
        let mut child = Command::new("cmd.exe")
            .args(["/c", "exit", "0"])
            .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED)
            .spawn()
            .unwrap();
        // A suspended process stays alive until the test explicitly terminates it.
        let target = Process::open(child.id(), false).unwrap();
        assert!(!native::wait(&stop, Some(&target), 20).unwrap());
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(native::wait(&stop, Some(&target), 1000).unwrap());
        let current = Process::open(std::process::id(), false).unwrap();
        assert_ne!(
            wind_common::locale::ready_event(target.pid, target.created),
            wind_common::locale::ready_event(current.pid, current.created)
        );
        assert!(!native::wait(&stop, Some(&current), 20).unwrap());
        unsafe {
            SetEvent(stop.0);
        }
        assert!(native::wait(&stop, Some(&current), 1000).unwrap());
    }
    #[test]
    fn module_presence_is_not_readiness_and_event_tracks_process_lifetime() {
        let current = Process::open(std::process::id(), false).unwrap();
        assert!(!current.ready());
        let name = native::wide(wind_common::locale::ready_event(
            current.pid,
            current.created,
        ));
        let event = Handle::checked(
            unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) },
            "ready event",
        )
        .unwrap();
        assert!(!current.ready());
        unsafe {
            SetEvent(event.0);
        }
        assert!(current.ready());
        drop(event);
        assert!(!current.ready());
    }
}
