# CI integration

The shared pipeline and repository dispatch are on `divvun-actions/main` at
[commit ab7728d](https://github.com/divvun/divvun-actions/commit/ab7728d6e562fff7d8d626d50513e62009f59a34).
The managed bootstrap can now select Wind. No Buildkite webhook was registered
for Wind when checked; hosted pipeline/webhook registration is still pending.

Wind uses the shared Divvun Actions Buildkite integration. Pipeline selection
and job orchestration live in `divvun/divvun-actions`, not in a `.buildkite`
file in this repository.

The managed bootstrap is defined in
`divvun-actions/services/sync-github/types.ts`:

```yaml
steps:
  - command: divvun-actions ci
    plugins:
      - ssh://git@github.com/divvun/divvun-actions.git#main: ~
```

`cli.ts` selects `pipelineDivvunWind()` for the `divvun-wind` repository.
`pipelines/divvun-wind.ts` schedules the Windows x64 job and attaches the shared
plugin at the generating actions revision, matching the other Divvun pipelines.
It imports the complete MSVC environment using `msvc-env x64`, then runs
`scripts/ci.ps1 -Build` from the Wind checkout. Failed setup, checks or builds
stop the job before artifact upload.

The job uploads the daemon, symbol helper, keyboard label DLL and PDB files as
unsigned Buildkite artifacts. No signing, installer or release publication job
is enabled yet. Outto packaging remains governed by the
[packaging contract](../packaging/README.md).

The shared Windows image uses Server Core. Its tests provide build and
noninteractive runtime coverage; Windows 11-only tests skip on that OS. An
actual Windows 11 Explorer/menu test needs an interactive VM, and the DLL smoke
test in `scripts/smoke-dll.ps1` is for that environment.

The managed bootstrap selects this pipeline from divvun-actions `main`. Repository/pipeline registration and
webhooks use `services/sync-github` scoped to `divvun/divvun-wind`; keep the
Buildkite pipeline private like its GitHub repository. Removing the old local
YAML alone does not activate hosted CI.

To test pipeline selection in the actions checkout:

```powershell
deno test --frozen --allow-env --allow-read pipelines/divvun-wind_test.ts
```

These tests exercise the real `divvun-actions ci` dispatcher and validate the
generated Buildkite schema without permission to spawn jobs or contact Buildkite.

Validation of this correction: Deno formatting, lint and both pipeline dispatch
tests pass. The generated MSVC setup/build command was executed locally and
passed strict Clippy, all 23 Rust tests and the release build. Artifact upload
and hosted Buildkite execution have not been exercised locally.
