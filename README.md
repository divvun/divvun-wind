# Divvun Wind

**Divvun Windows Daemon** provides persistent Windows desktop capabilities.
Its first capability supplies localized keyboard names and taskbar abbreviations
for languages Windows does not recognize.

This is the initial standalone implementation for **Windows 11 x64**. Normal
daemon/helper launch and direct DLL loading are inert on Windows 10 and Server.
Actual Windows 10 VM verification and the installer remain release gates.

## Build and check

Use Windows with an x64 MSVC developer environment, a Windows SDK, Git and rustup.
Rust 1.98.1 is pinned in `rust-toolchain.toml`. Cargo fetches the pinned Detours
revision and its native source submodule; no sibling checkout is required.

```powershell
pwsh -NoProfile -File scripts/ci.ps1 -Build
```

This checks formatting, Clippy and tests, then builds these unsigned files in
`target/x86_64-pc-windows-msvc/release/`:

- `divvun-wind.exe`: one daemon per interactive user session.
- `divvun_keyboard_labels.dll`: Explorer keyboard label hooks.
- `divvun-wind-symbols.exe`: exact-image Microsoft symbol preparation/validation.

`.buildkite/pipeline.yml` runs the same checks on the Windows queue. Provisioning
and pinning the release builder image is still required before release CI.

## Keyboard definitions

Each keyboard installer owns one or more uniquely named JSON files immediately
under `%ProgramData%\Divvun\Keyboards\`. The filename has no language semantics.
For example, any uniquely named file may contain:

```json
{
  "sjd-cyrl": {
    "abbreviation": "КИЛ",
    "display_name": {
      "en": "Kildin Sami",
      "ru": "Кильдинский саамский"
    }
  }
}
```

Names follow the calling thread's UI languages, then parent tags, then English.
An unmatched translation preserves Windows' label. Conflicting definitions
disable only the affected tag; identical definitions coalesce. Bad files produce
diagnostics while valid files remain usable. Missing/empty directories are valid.

Validation commands perform no injection and require no symbol files:

```powershell
target/x86_64-pc-windows-msvc/release/divvun-wind.exe check-file tests/fixtures/localized.json
target/x86_64-pc-windows-msvc/release/divvun-wind.exe check --directory tests/fixtures
```

Commands emit JSON and return nonzero on validation errors. On Windows 10 all
commands currently return successful no-op before parsing arguments.

## Runtime status

`divvun-wind run` monitors the current session's Explorer, waits for its taskbar
modules, resolves matching symbols and injects the DLL once per Explorer
lifetime. `status` reports daemon and DLL initialization; `stop` stops the daemon.
The DLL remains pinned until Explorer exits. Starting this development runtime
is an explicit action; building and testing do not start it or replace a running
prototype.

The DLL reads the catalogue on its initialization worker. Data changes take
effect on the next Explorer start. The idle daemon rescans so definitions added
after daemon startup can trigger the first injection. Shared loader recovery
exists, but DLL live reload and repeatable UI refresh are not wired yet.
Readiness acknowledges initialization, not visible taskbar/menu refresh.

Logs, status and downloaded symbol caches belong in `%LOCALAPPDATA%\Divvun\Wind`;
keyboard definitions belong in ProgramData. Symbol preparation expects the
declared debugger runtime beside the helper, or an explicit `--dbghelp` argument
for development. The unsigned build alone is not an installed runtime bundle.

Installation will use **outto**, through **divvun-actions**. See the
[implementation status](docs/status.md), [catalogue design](docs/keyboard-label-catalogue.md)
and [packaging contract](packaging/README.md).

## License and origin

MIT OR Apache-2.0, following the prototype workspace's declared license.
See [origin and dependency notices](docs/origin.md).
