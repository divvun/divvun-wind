# Outto packaging

`product.json` is the product contract for the future divvun-actions bundler.
The application version comes from `workspace.package.version` in Cargo.toml.
It is not an outto manifest: the `*_revision` fields are build inputs and must
not be sent to outto as unsupported configuration options.

Use the existing `OuttoBuilder` and `makeOuttoInstaller` in divvun-actions.
The inspected revisions and required integration points are recorded in the
[packaging plan](../docs/keyboard-label-build-and-packaging.md).

The initial CI job produces unsigned runtime binaries. Installer generation and
distribution remain pending because the reviewed outto revision does not yet
implement the required Windows 10 no-op before UI/elevation/side effects. It
also needs reliable required-action failure and rollback semantics. A runtime
OS guard or an unenforced `min_version` field does not resolve those gaps.

Before adding the bundler:

- Resolve outto eligibility and lifecycle semantics and pin the resulting toolkit.
- Implement all-session startup, versioned updates and removal coordination.
- Declare the licensed SDK debugger runtime artifact (dbghelp.dll/symsrv.dll),
  its version and checksums; do not copy a developer's installed SDK into CI.
- Sign runtime files before embedding them; then sign and verify the installer.
- Test install/update/uninstall on Windows 11 and successful no-op on Windows 10.
- Make artifact publication depend on every required verification job.

The runtime package must preserve the keyboard catalogue and must not install
the test fixtures as real keyboard definitions.
