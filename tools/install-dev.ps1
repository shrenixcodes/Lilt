<#
.SYNOPSIS
    Checks, builds, installs, and starts the local Lilt development build.
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path -LiteralPath $cargo)) { throw "Cargo not found at $cargo" }

Push-Location $root
try {
    $checks = @(
        @('fmt', '--check'),
        @('clippy', '--features', 'dev-build', '--all-targets', '--', '-D', 'warnings'),
        @('test', '--features', 'dev-build'),
        @('build', '--release', '--features', 'dev-build', '--locked')
    )
    foreach ($arguments in $checks) {
        Write-Host "Running cargo $($arguments -join ' ')"
        & $cargo @arguments
        if ($LASTEXITCODE -ne 0) { throw "cargo $($arguments[0]) failed with exit code $LASTEXITCODE" }
    }

    $built = Join-Path $root 'target\release\lilt.exe'
    $installDir = Join-Path $env:LOCALAPPDATA 'Programs\Lilt Dev'
    $installed = Join-Path $installDir 'lilt-dev.exe'
    $running = @(Get-CimInstance Win32_Process -Filter "name = 'lilt-dev.exe'" |
        Where-Object { $_.ExecutablePath -eq $installed })
    foreach ($process in $running) {
        Stop-Process -Id $process.ProcessId -Force -ErrorAction Stop
        for ($attempt = 0; $attempt -lt 60; $attempt++) {
            if (-not (Get-CimInstance Win32_Process -Filter "ProcessId = $($process.ProcessId)")) { break }
            Start-Sleep -Milliseconds 500
        }
        if (Get-CimInstance Win32_Process -Filter "ProcessId = $($process.ProcessId)") {
            throw "Lilt Dev process $($process.ProcessId) did not stop"
        }
    }

    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    Copy-Item -LiteralPath $built -Destination $installed -Force

    $shortcutPath = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Lilt Dev.lnk'
    $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
    $shortcut.TargetPath = $installed
    $shortcut.WorkingDirectory = $installDir
    $shortcut.Save()

    Start-Process -FilePath $installed
    Write-Host "Installed and started $installed"
} finally {
    Pop-Location
}
