# Outto installer

`product.json` is the input to `actions/divvun-wind/bundle.ts` in divvun-actions.
The bundler uses `OuttoBuilder` and `makeOuttoInstaller`; outto owns installation,
registry/file tracking, upgrades and the installed uninstaller.

Get `divvun-wind_x86-64-pc-windows-msvc_<version>.exe` from the private repository's
[dev-latest release](https://github.com/divvun/divvun-wind/releases/tag/dev-latest)
after a successful main build. Version tags publish versioned GitHub releases.
See [CI details](../docs/ci.md) for signing and development artifacts.

The package includes `divvun-wind.exe`, `divvunwind.dll`, the symbol helper,
Microsoft's x64 `dbghelp.dll`, `msdia140.dll` and `symsrv.dll`, and license notices. Microsoft
packages are pinned by version/URL/SHA-256 and their signatures are checked.
No developer SDK installation is copied into the package.

The distributed launcher checks the shared Windows 11 client eligibility rule
before extracting or starting outto. Windows 10/Server return success without
installer UI, elevation or extraction. Outto remains the installer; the launcher
only supplies the early gate and forwards arguments/exit status. Its raw Cargo
build has no embedded installer and is not a distribution artifact.

Outto installs into `Program Files\Divvun\Wind\versions\<version>`. It registers
the `DivvunWind` value in the machine Run key, so each interactive user starts
the daemon at logon, including users created after installation. The Start menu
also has a Divvun Wind entry. **Sign out and back in after installing or updating**
to activate the version in each session. Automated handover of running sessions
is not implemented yet.

Versioned paths avoid overwriting an older DLL loaded in Explorer. Restart
Manager is disabled so installation does not close Explorer to unlock files.
Outto owns only Wind's Run value, its runtime files and shortcut. The shared
`ProgramData\Divvun\Keyboards` directory is preserved; keyboard installers
continue to own their JSON files. Wind does not install demonstration definitions.

This is a development installer. Clean Windows 11/10 VM install/update/uninstall,
two-user startup, rollback and cleanup of files still loaded during removal need
verification. Sign out of other sessions and stop the current daemon before
removal; hooks already loaded in Explorer last until it exits. Automatic session
handover and deferred cleanup remain follow-up work, not claims of this package.
