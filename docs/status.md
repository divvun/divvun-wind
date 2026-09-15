# Implementation status

Initial extraction, 2026-09-15. This file distinguishes implemented code from the
longer catalogue and packaging plans.

## Implemented in this repository

- Independent four-crate Cargo workspace, pinned Rust/Detours, lockfile and
  local check/build script. The managed Buildkite pipeline and repository dispatch
  live in divvun-actions; see [CI integration](ci.md).
- Shared manifest-independent OS build/client check. Daemon, symbol helper and
  DLL entry points perform a successful no-op on ineligible systems.
- Shared directory catalogue loader: UTF-8/BOM, grammar-based tag validation,
  localization, duplicate-member rejection, conflict isolation, bounds and
  source diagnostics. Reparse-point candidates are not followed.
- Loader snapshots retain stale contributions for damaged existing files and
  preserve the previous full view on scan failure. Confirmed deletions remove
  contributions. The DLL currently uses only its initial snapshot.
- Daemon validation CLI, empty-catalogue idle/rescan behavior, process-lifetime
  supervision and injection, exact-image symbol preparation and label hooks.
- DLL has no experimental RPC interface and remains pinned until process exit.
  It performs no detach-time joins or hook transactions. Empty initialization
  acknowledges readiness without installing hooks.

## Verification completed

On this Windows 11 x64 development machine:

- `cargo fmt --all -- --check` and Clippy for all workspace targets with
  `-D warnings` pass.
- `cargo test --workspace --locked` passes all 23 tests. These include isolated
  native en/ru hooks, symbol/cache validation, process readiness, catalogue
  schema/conflicts/recovery/bounds, and CLI JSON/error behavior. The conditional
  unsupported-OS CLI case requires a Windows 10/Server host to exercise its body.
- `cargo build --workspace --release --locked` produces both EXEs and the DLL.
- `scripts/smoke-dll.ps1` loads the built DLL into a disposable process with an
  empty temporary catalogue and observes its readiness acknowledgement.

This is local build/runtime evidence. Clean VM, actual Explorer UI, Windows 10
side effects and the hosted Buildkite run remain unverified for this extraction.

## Next work and release gates

1. Implement outto's successful early unsupported-OS skip, required helper
   failure/rollback semantics and owned JSON publication contract; then add the
   divvun-actions outto bundler and pinned toolkit provisioning.
2. Implement all-session startup, versioned update/removal coordination and
   explicit migration from the prototype's single shared JSON file.
3. Add shell crash-loop protection and production initialization/error reporting.
   Harden/review concurrent hook installation before distributing the DLL.
4. Verify the built runtime in disposable Windows 11/10 VMs, including actual
   taskbar/menu behavior, two users, logon/cold boot and Explorer replacement.
5. Sign, package with outto, verify the actual installer and wire gated release
   publication. The current pipeline has no signing or publication job.
6. Add DLL catalogue reload/acknowledgement and repeatable UI refresh, with
   independent catalogue-generation and visible-refresh status. Add bounded
   scan-race reconciliation for runtime reload. These are not implemented yet.

The running prototype is unchanged. This repository has not registered startup,
migrated ProgramData, restarted Explorer or injected the new DLL.

The OS query uses the layout documented by Microsoft for
[RTL_OSVERSIONINFOEXW](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdm/ns-wdm-_osversioninfoexw).
The build floor is an OS eligibility check; each shell image must also pass its
own exact-symbol validation.
