<#
.SYNOPSIS
    Measures Lilt in its main states with measure.ps1.

.DESCRIPTION
    Scenarios:
      idle      Compact island, nothing interacting with it.
      expanded  Expanded island, so the clock, agenda and system readings are live.
      toggle    Expanded and collapsed every two seconds.
      media     Compact island while a test media session changes track every five
                seconds (requires tools/media-session to be built).
      soak      Compact island for 30 minutes, to check memory stays flat.

    Expansion is triggered by posting Lilt's global-shortcut message to its control
    window, so no input is simulated and the pointer is not used.

.EXAMPLE
    .\benchmarks\scenarios.ps1 -Scenarios idle,expanded,toggle -OutFile benchmarks\results.json
#>
[CmdletBinding()]
param(
    # Defaults to the release build in this repository.
    [string] $Exe,
    [string[]] $Scenarios = @('idle', 'expanded', 'toggle', 'media'),
    [int] $SampleSeconds = 120,
    [string] $OutFile
)

$ErrorActionPreference = 'Stop'
# "powershell -File" passes a list as one comma-separated string.
$Scenarios = @($Scenarios | ForEach-Object { $_ -split ',' } | Where-Object { $_ })
# $PSScriptRoot is not available in parameter defaults on Windows PowerShell 5.1.
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $Exe) { $Exe = Join-Path $here '..\target\release\lilt.exe' }
$measure = Join-Path $here 'measure.ps1'
$mediaTool = Join-Path $here '..\tools\media-session\target\release\media-session.exe'

if (-not ('LiltScenario.Native' -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
namespace LiltScenario {
    public static class Native {
        [DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern IntPtr FindWindow(string cls, string title);
        [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr hwnd, uint msg, IntPtr w, IntPtr l);
        // WM_HOTKEY with Lilt's shortcut identifier.
        public static void ToggleIsland() { PostMessage(FindWindow("Lilt.Control", null), 0x0312, (IntPtr)1, IntPtr.Zero); }
    }
}
"@
}

Get-Process lilt -ErrorAction SilentlyContinue | Stop-Process -Force
$common = @{ Exe = $Exe; SampleSeconds = $SampleSeconds }
if ($OutFile) { $common.OutFile = $OutFile }

foreach ($scenario in $Scenarios) {
    switch ($scenario) {
        'idle' { & $measure @common -Label 'idle' }
        'expanded' { & $measure @common -Label 'expanded' -Prepare { [LiltScenario.Native]::ToggleIsland() } }
        'toggle' { & $measure @common -Label 'toggle every 2 s' -Repeat { [LiltScenario.Native]::ToggleIsland() } -RepeatSeconds 2 }
        'media' {
            if (-not (Test-Path $mediaTool)) { Write-Warning "build tools/media-session first"; continue }
            $session = Start-Process $mediaTool -ArgumentList 5, 6 -WindowStyle Hidden -PassThru
            try { & $measure @common -Label 'track change every 5 s' }
            finally { Stop-Process -Id $session.Id -ErrorAction SilentlyContinue }
        }
        'soak' { & $measure -Exe $Exe -Label 'soak 30 min' -SampleSeconds 1800 -IntervalSeconds 10 -OutFile $OutFile }
        default { Write-Warning "unknown scenario $scenario" }
    }
    Start-Sleep -Seconds 3
}
