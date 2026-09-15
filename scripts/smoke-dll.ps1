# Run in a dedicated PowerShell process: the DLL remains pinned until it exits.
param([string]$Dll = 'target/x86_64-pc-windows-msvc/release/divvun_keyboard_labels.dll')
$ErrorActionPreference = 'Stop'
$dllPath = (Resolve-Path -LiteralPath $Dll).Path
Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class WindDllSmoke {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr LoadLibraryExW(string path, IntPtr file, uint flags);
}
'@

$dataRoot = Join-Path ([IO.Path]::GetTempPath()) ('divvun-wind-smoke-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $dataRoot | Out-Null
$previousProgramData = $env:ProgramData
try {
    # Child process only. The real machine-wide catalogue is never used/changed.
    $env:ProgramData = $dataRoot
    $process = [Diagnostics.Process]::GetCurrentProcess()
    $created = $process.StartTime.ToFileTimeUtc()
    $eventName = 'Local\Divvun.Wind.KeyboardLabels.Ready.{0}.{1:X16}' -f $PID, $created
    $module = [WindDllSmoke]::LoadLibraryExW($dllPath, [IntPtr]::Zero, 0x1100)
    if ($module -eq [IntPtr]::Zero) { throw "DLL load failed: $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    $ready = $null
    while ([DateTime]::UtcNow -lt $deadline) {
        if ([Threading.EventWaitHandle]::TryOpenExisting($eventName, [ref]$ready)) {
            try {
                if ($ready.WaitOne(0)) {
                    Write-Output '{"empty_catalogue_initialization":"ready"}'
                    exit 0
                }
            } finally { $ready.Dispose() }
        }
        Start-Sleep -Milliseconds 50
    }
    throw 'DLL did not acknowledge empty-catalogue initialization within 10 seconds (this smoke test requires Windows 11)'
} finally {
    $env:ProgramData = $previousProgramData
    # Only an empty, uniquely created test directory is removed; no recursion.
    Remove-Item -LiteralPath $dataRoot
}
