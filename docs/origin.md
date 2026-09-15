# Source origin and dependencies

The initial runtime was extracted on 2026-09-15 from `win-a11y-mcp`, whose
workspace declares `MIT OR Apache-2.0`. That checkout had no Git history or
separate license files. The original source-file SHA-256 hashes are recorded
in `prototype-source-hashes.json`; they identify inputs before extraction edits.
The catalogue/build design documents and localized test fixture also originate
from that workspace. New code uses the same license choice.

Only the locale watcher, label hooks, UI language selection, taskbar symbol
resolver/cache validation and their required data contracts were extracted.
MCP, RPC pipes, arbitrary function execution, heartbeat/ejection machinery and
the general accessibility daemon are not included.

`windows-detours` is pinned to published revision
`af6fab2ab3c01da2947905307e562fa246a8fcfb` at
<https://github.com/necessary-nu/windows-detours>, declared Apache-2.0 OR MIT,
authored by Brendan Molloy. Its native Microsoft Detours submodule is pinned to
`9764cebcb1a75940e68fa83d6730ffaf0f669401`, under the MIT license, at
<https://github.com/microsoft/Detours>.

The prototype's local wrapper revision `7776f5353eefcdf991d794f05251a32fce388d4f`
was not fetchable from its remote. The published revision above was inspected
for its raw hook/transaction API and MSVC build. The local checkout's GNU build
changes are not needed or copied into this x64 MSVC build.

Cargo.lock records all remaining Rust dependency versions and registry checksums.
Distribution must retain applicable third-party notices, including Detours, and
the license terms of the debugger runtime selected for packaging.
