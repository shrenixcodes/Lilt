<#
.SYNOPSIS
    Measures the resource usage of a desktop executable and every process it spawns.

.DESCRIPTION
    Launches the executable, records the time until it shows a visible top-level window,
    waits for the process tree to settle, then samples it for a fixed duration.

    Reported per run (summed over the whole process tree, e.g. including WebView2 helpers):
      - Startup time to first visible window.
      - Private working set (Task Manager "Memory" column), working set and private bytes.
      - CPU time as a share of all logical processors and of a single logical processor.
      - Peak CPU over a single sampling interval.
      - Context switches per second (a proxy for wake-ups).
      - GPU engine utilisation reported by the "GPU Engine" performance counters.
      - Disk and other I/O transferred during the sampling window.
      - Remote TCP endpoints observed (network activity check).

    Performance counter names are localised by Windows; the GPU counter requires an
    English counter set. All other metrics use WMI/CIM classes and work on any locale.

.EXAMPLE
    .\benchmarks\measure.ps1 -Exe .\target\release\lilt.exe -Label idle -SampleSeconds 120
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)] [string] $Exe,
    [string] $Label,
    [string[]] $Arguments = @(),
    [hashtable] $Environment = @{},
    [int] $SettleSeconds = 10,
    [int] $SampleSeconds = 60,
    [int] $IntervalSeconds = 2,
    [string] $OutFile
)

$ErrorActionPreference = 'Stop'
$Exe = (Resolve-Path $Exe).Path
if (-not $Label) { $Label = [IO.Path]::GetFileNameWithoutExtension($Exe) }

if (-not ('LiltBench.Windows' -as [type])) {
    Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
namespace LiltBench {
    public static class Windows {
        delegate bool EnumProc(IntPtr hwnd, IntPtr lparam);
        [DllImport("user32.dll")] static extern bool EnumWindows(EnumProc callback, IntPtr lparam);
        [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr hwnd);
        [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr hwnd, out uint pid);
        public static bool HasVisibleWindow(HashSet<uint> pids) {
            bool found = false;
            EnumWindows((hwnd, l) => {
                uint pid;
                GetWindowThreadProcessId(hwnd, out pid);
                if (pids.Contains(pid) && IsWindowVisible(hwnd)) { found = true; return false; }
                return true;
            }, IntPtr.Zero);
            return found;
        }
    }
}
"@
}

function Get-ProcessTree([int] $RootId) {
    $all = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, Name, ReadTransferCount, WriteTransferCount, OtherTransferCount)
    $ids = [System.Collections.Generic.List[int]]::new()
    $ids.Add($RootId)
    for ($i = 0; $i -lt $ids.Count; $i++) {
        foreach ($p in $all) {
            if ($p.ParentProcessId -eq $ids[$i] -and -not $ids.Contains([int]$p.ProcessId)) { $ids.Add([int]$p.ProcessId) }
        }
    }
    $all | Where-Object { $ids.Contains([int]$_.ProcessId) }
}

function Get-CpuSeconds([int[]] $Ids) {
    $map = @{}
    foreach ($p in Get-Process -Id $Ids -ErrorAction SilentlyContinue) { $map[$p.Id] = $p.TotalProcessorTime.TotalSeconds }
    $map
}

function Get-ContextSwitches([int[]] $Ids) {
    # Cumulative per-thread counters keyed by thread ID, so threads that exit or start
    # during the window do not distort the difference.
    $map = @{}
    foreach ($t in Get-CimInstance Win32_PerfRawData_PerfProc_Thread -Property IDProcess, IDThread, ContextSwitchesPersec) {
        if ($Ids -contains [int]$t.IDProcess) { $map["$($t.IDProcess):$($t.IDThread)"] = [uint64]$t.ContextSwitchesPersec }
    }
    $map
}

function Get-ContextSwitchDelta($Before, $After) {
    $delta = [uint64]0
    foreach ($key in $After.Keys) {
        $start = [uint64]0
        if ($Before.ContainsKey($key)) { $start = $Before[$key] }
        if ($After[$key] -ge $start) { $delta += $After[$key] - $start }
    }
    $delta
}

function Get-Io($Tree) {
    $sum = @{ Read = [uint64]0; Write = [uint64]0; Other = [uint64]0 }
    foreach ($p in $Tree) {
        $sum.Read += [uint64]$p.ReadTransferCount
        $sum.Write += [uint64]$p.WriteTransferCount
        $sum.Other += [uint64]$p.OtherTransferCount
    }
    $sum
}

function Get-Memory([int[]] $Ids) {
    $m = @{ PrivateWs = [uint64]0; Ws = [uint64]0; PrivateBytes = [uint64]0 }
    foreach ($p in Get-CimInstance Win32_PerfFormattedData_PerfProc_Process -Property IDProcess, WorkingSetPrivate, WorkingSet, PrivateBytes) {
        if ($Ids -contains [int]$p.IDProcess) {
            $m.PrivateWs += [uint64]$p.WorkingSetPrivate
            $m.Ws += [uint64]$p.WorkingSet
            $m.PrivateBytes += [uint64]$p.PrivateBytes
        }
    }
    $m
}

function Get-GpuPercent([int[]] $Ids) {
    try {
        $samples = (Get-Counter -Counter '\GPU Engine(*)\Utilization Percentage' -ErrorAction Stop).CounterSamples
    } catch { return $null }
    $total = 0.0
    foreach ($s in $samples) {
        if ($s.InstanceName -match '^pid_(\d+)_' -and $Ids -contains [int]$Matches[1]) { $total += $s.CookedValue }
    }
    $total
}

function Get-RemoteEndpoints([int[]] $Ids) {
    $found = @()
    foreach ($c in Get-NetTCPConnection -ErrorAction SilentlyContinue) {
        if ($Ids -contains [int]$c.OwningProcess -and $c.RemoteAddress -notmatch '^(0\.0\.0\.0|127\.|::1?$|::$)') {
            $found += "$($c.RemoteAddress):$($c.RemotePort)"
        }
    }
    $found
}

$saved = @{}
foreach ($k in $Environment.Keys) {
    $saved[$k] = [Environment]::GetEnvironmentVariable($k, 'Process')
    [Environment]::SetEnvironmentVariable($k, [string]$Environment[$k], 'Process')
}

$cores = [Environment]::ProcessorCount
$watch = [Diagnostics.Stopwatch]::StartNew()
$startArgs = @{ FilePath = $Exe; PassThru = $true }
if ($Arguments.Count -gt 0) { $startArgs.ArgumentList = $Arguments }
$root = Start-Process @startArgs

foreach ($k in $saved.Keys) { [Environment]::SetEnvironmentVariable($k, $saved[$k], 'Process') }

$startupMs = $null
$pidSet = [System.Collections.Generic.HashSet[uint32]]::new()
while ($watch.Elapsed.TotalSeconds -lt 20) {
    $pidSet.Clear()
    foreach ($p in Get-ProcessTree $root.Id) { [void]$pidSet.Add([uint32]$p.ProcessId) }
    if ([LiltBench.Windows]::HasVisibleWindow($pidSet)) { $startupMs = [math]::Round($watch.Elapsed.TotalMilliseconds); break }
    Start-Sleep -Milliseconds 5
}

Start-Sleep -Seconds $SettleSeconds

$tree = @(Get-ProcessTree $root.Id)
$ids = @($tree | ForEach-Object { [int]$_.ProcessId })
$cpuStart = Get-CpuSeconds $ids
$ioStart = Get-Io $tree
$csStart = Get-ContextSwitches $ids
$sampleWatch = [Diagnostics.Stopwatch]::StartNew()

$memSamples = @()
$gpuSamples = @()
$cpuPeak = 0.0
$endpoints = @{}
$prevCpu = ($cpuStart.Values | Measure-Object -Sum).Sum
$prevTime = 0.0

while ($sampleWatch.Elapsed.TotalSeconds -lt $SampleSeconds) {
    $tick = $sampleWatch.Elapsed.TotalSeconds
    $tree = @(Get-ProcessTree $root.Id)
    $ids = @($tree | ForEach-Object { [int]$_.ProcessId })
    $memSamples += Get-Memory $ids
    $gpu = Get-GpuPercent $ids
    if ($null -ne $gpu) { $gpuSamples += $gpu }
    foreach ($e in Get-RemoteEndpoints $ids) { $endpoints[$e] = $true }

    $now = $sampleWatch.Elapsed.TotalSeconds
    $cpuNow = ((Get-CpuSeconds $ids).Values | Measure-Object -Sum).Sum
    if ($now -gt $prevTime) {
        $intervalPct = ($cpuNow - $prevCpu) / ($now - $prevTime) / $cores * 100
        if ($intervalPct -gt $cpuPeak) { $cpuPeak = $intervalPct }
    }
    $prevCpu = $cpuNow
    $prevTime = $now

    $remaining = $IntervalSeconds - ($sampleWatch.Elapsed.TotalSeconds - $tick)
    if ($remaining -gt 0) { Start-Sleep -Milliseconds ([int]($remaining * 1000)) }
}

$elapsed = $sampleWatch.Elapsed.TotalSeconds
$tree = @(Get-ProcessTree $root.Id)
$ids = @($tree | ForEach-Object { [int]$_.ProcessId })
$cpuEnd = Get-CpuSeconds $ids
$ioEnd = Get-Io $tree
$csEnd = Get-ContextSwitches $ids

$cpuDelta = 0.0
foreach ($id in $cpuEnd.Keys) {
    $before = 0.0
    if ($cpuStart.ContainsKey($id)) { $before = $cpuStart[$id] }
    $cpuDelta += $cpuEnd[$id] - $before
}

foreach ($id in ($ids | Sort-Object -Descending)) { Stop-Process -Id $id -Force -ErrorAction SilentlyContinue }

function Get-Mb($values) { [math]::Round((($values | Measure-Object -Average).Average) / 1MB, 1) }
function Get-MaxMb($values) { [math]::Round((($values | Measure-Object -Maximum).Maximum) / 1MB, 1) }

$result = [ordered]@{
    label                 = $Label
    exe                   = [IO.Path]::GetFileName($Exe)
    environment           = ($Environment.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join ' '
    processes             = $tree.Count
    processNames          = (($tree | ForEach-Object { $_.Name }) | Group-Object | ForEach-Object { "$($_.Name)x$($_.Count)" }) -join ', '
    startupMs             = $startupMs
    sampleSeconds         = [math]::Round($elapsed, 1)
    privateWsMbAvg        = Get-Mb ($memSamples | ForEach-Object { $_.PrivateWs })
    privateWsMbMax        = Get-MaxMb ($memSamples | ForEach-Object { $_.PrivateWs })
    workingSetMbAvg       = Get-Mb ($memSamples | ForEach-Object { $_.Ws })
    privateBytesMbAvg     = Get-Mb ($memSamples | ForEach-Object { $_.PrivateBytes })
    cpuPctOfTotal         = [math]::Round($cpuDelta / $elapsed / $cores * 100, 3)
    cpuPctOfOneCore       = [math]::Round($cpuDelta / $elapsed * 100, 3)
    cpuPeakPctOfTotal     = [math]::Round($cpuPeak, 3)
    contextSwitchesPerSec = [math]::Round((Get-ContextSwitchDelta $csStart $csEnd) / $elapsed, 1)
    gpuPctAvg             = if ($gpuSamples.Count) { [math]::Round(($gpuSamples | Measure-Object -Average).Average, 3) } else { $null }
    gpuPctMax             = if ($gpuSamples.Count) { [math]::Round(($gpuSamples | Measure-Object -Maximum).Maximum, 3) } else { $null }
    ioReadKb              = [math]::Round(($ioEnd.Read - $ioStart.Read) / 1KB, 1)
    ioWriteKb             = [math]::Round(($ioEnd.Write - $ioStart.Write) / 1KB, 1)
    ioOtherKb             = [math]::Round(($ioEnd.Other - $ioStart.Other) / 1KB, 1)
    remoteEndpoints       = @($endpoints.Keys)
}

if ($OutFile) {
    # Windows PowerShell 5.1 returns a JSON array as a single object; enumerate it explicitly.
    $existing = [System.Collections.Generic.List[object]]::new()
    if (Test-Path $OutFile) {
        foreach ($entry in (Get-Content $OutFile -Raw | ConvertFrom-Json)) { $existing.Add($entry) }
    }
    $existing.Add([pscustomobject]$result)
    ConvertTo-Json -InputObject $existing.ToArray() -Depth 4 | Out-File -FilePath $OutFile -Encoding utf8
}

[pscustomobject]$result
