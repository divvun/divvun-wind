# CI integration

The shared pipeline and repository dispatch include the PowerShell quoting fix at
[commit b7b62ee](https://github.com/divvun/divvun-actions/commit/b7b62eeda223dcc91132638431de4c8baeb1cc8e).
The first hosted build reached the Windows job but failed while parsing its inline
PowerShell command. The job now invokes `pwsh -NoProfile -File scripts/buildkite.ps1`.

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
The Wind `scripts/buildkite.ps1` entry point imports the complete MSVC environment
using `msvc-env x64`, then runs `scripts/ci.ps1 -Build`. PowerShell syntax and
artifact globs stay in the script, outside Buildkite command interpolation and
shell quoting. Failed setup, checks, builds or artifact uploads stop the job.

The job uploads the daemon, symbol helper, `divvunwind.dll` and PDB files as
unsigned Buildkite artifacts. No signing, installer or release publication job
is enabled yet. Outto packaging remains governed by the
[packaging contract](../packaging/README.md).

The shared Windows image uses Server Core. Its tests provide build and
noninteractive runtime coverage; Windows 11-only tests skip on that OS. An
actual Windows 11 Explorer/menu test needs an interactive VM, and the DLL smoke
test in `scripts/smoke-dll.ps1` is for that environment.

The managed bootstrap selects this pipeline from divvun-actions `main`. Repository/pipeline registration and
webhooks use `services/sync-github` scoped to `divvun/divvun-wind`; keep the
Buildkite pipeline private like its GitHub repository. Start a new build at the updated Wind commit so both the new script and shared
pipeline are used. Retrying an already generated Windows job retains its old command.

To test pipeline selection in the actions checkout:

```powershell
deno test --frozen --allow-env --allow-read pipelines/divvun-wind_test.ts
```

These tests exercise the real `divvun-actions ci` dispatcher and validate the
generated Buildkite schema without permission to spawn jobs or contact Buildkite.

Validation of this correction: Deno formatting, lint and both pipeline dispatch
tests pass. The script was invoked locally through a PowerShell command wrapper
and passed strict Clippy, all 23 Rust tests and the release build. Failure checks
with stubbed commands confirmed that setup/check failures prevent artifact upload
and that an upload failure stops further uploads. Artifact arguments, including the
literal PDB glob, were captured with an upload stub; hosted uploads remain unverified.
