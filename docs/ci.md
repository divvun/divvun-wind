# CI and installer releases

Wind uses the managed Divvun Actions Buildkite bootstrap:

```yaml
steps:
  - command: divvun-actions ci
    plugins:
      - ssh://git@github.com/divvun/divvun-actions.git#main: ~
```

The shared `pipelines/divvun-wind.ts` dispatches three dependent steps:

1. Windows x64: import MSVC, format/check/test, build the runtime and upload binaries/PDBs.
2. Windows x64: download those binaries, stage the pinned Microsoft debugger DLLs,
   build the outto installer, and embed it in Wind's Windows 11 launcher.
3. Linux: verify the installer hash/build identity, then publish the installer,
   generated outto manifest, build provenance and `SHA256SUMS` to GitHub Releases.

Successful `main` builds update the **`dev-latest` prerelease**. A `vX.Y.Z` tag
matching the Cargo workspace version creates a versioned release. Pull requests,
other branches and non-version tags build explicitly `.UNSIGNED.exe` artifacts
in Buildkite and cannot publish. Signing failures fail the release build; there
is no unsigned fallback. Publication depends on both checks and packaging.

Signed builds use the existing Divvun signing service for the three runtime
binaries, outto's installer/uninstaller, and the outer launcher. Microsoft's
debugger DLL signatures are preserved and checked. The shared Windows image
provides outto; packaging requires its GUI and uninstaller and records their
SHA-256 hashes. NuGet dependency versions, URLs and hashes live in Wind metadata.

Download the installer while signed into a GitHub account with repository access:

- [Rolling development release](https://github.com/divvun/divvun-wind/releases/tag/dev-latest)
- Asset: **`divvun-wind-windows-x86_64.exe`**

```powershell
gh release download dev-latest --repo divvun/divvun-wind --pattern divvun-wind-windows-x86_64.exe
```

The managed bootstrap pins generated jobs to the actions revision that generated
them. New pipeline code requires a fresh build, rather than retrying an old job.
Hosted installer builds reached outto's uninstaller signing, then failed because
outto's Windows command construction escaped embedded quotes for the wrong parser.
The shared signing adapter now expands quoted paths from the child environment,
after that escaping step. This correction is tested through the installed outto
CLI with spaced signer/output paths and a failing test signer. The next hosted
build will exercise the real signing service and release publication.

Local validation: all 25 Rust tests, strict Clippy and the release build pass;
the actual unsigned outto installer has been built with verified Microsoft DLLs.
Symbol resolution against the current Windows 11 shell succeeds with the full
pinned runtime, including `msdia140.dll` required by this DbgHelp version.
The shared pipeline's four tests cover release/PR/tag dispatch, dependency gates
and package ownership. Run them in divvun-actions with:

```powershell
deno test --frozen --allow-env --allow-read pipelines/divvun-wind_test.ts
```

With outto installed on Windows, also run the signing integration test:

```powershell
deno test --frozen --allow-env --allow-read --allow-write --allow-run=outto actions/outto/lib_test.ts
```

This test builds disposable fixtures with a test signer. It checks invocation
and failure propagation without using signing credentials or installing anything.


Server Core CI supplies build and noninteractive checks. Real Windows 11/10
installation, update, removal, logon and Explorer menu tests require VMs. See
[installer behavior and remaining verification](../packaging/README.md).
