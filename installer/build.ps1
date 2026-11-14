<#
.SYNOPSIS
    Builds the Lilt installer from an existing release build.

.DESCRIPTION
    Reads the version from Cargo.toml, runs NSIS on installer\lilt.nsi and writes
    target\installer\Lilt-<version>-x64-setup.exe with a .sha256 file beside it.
    Run `cargo build --release` first.
#>
[CmdletBinding()]
param(
    # Path to makensis.exe; found automatically when NSIS is installed in the default location.
    [string] $MakeNsis
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)

$manifest = Get-Content (Join-Path $root 'Cargo.toml') -Raw
if ($manifest -notmatch '(?m)^version\s*=\s*"(\d+\.\d+\.\d+)"') { throw 'version not found in Cargo.toml' }
$version = $Matches[1]

$exe = Join-Path $root 'target\release\lilt.exe'
if (-not (Test-Path $exe)) { throw "missing $exe; run 'cargo build --release' first" }

if (-not $MakeNsis) {
    $candidates = @(
        (Get-Command makensis.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
        (Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'),
        (Join-Path $env:ProgramFiles 'NSIS\makensis.exe')
    ) | Where-Object { $_ -and (Test-Path $_) }
    $MakeNsis = $candidates | Select-Object -First 1
}
if (-not $MakeNsis) { throw 'makensis.exe not found; install NSIS 3 from https://nsis.sourceforge.io' }

$output = Join-Path $root 'target\installer'
New-Item -ItemType Directory -Force $output | Out-Null
& $MakeNsis /V2 "/DVERSION=$version" (Join-Path $root 'installer\lilt.nsi')
if ($LASTEXITCODE -ne 0) { throw "makensis failed with exit code $LASTEXITCODE" }

$installer = Join-Path $output "Lilt-$version-x64-setup.exe"
$hash = (Get-FileHash $installer -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $(Split-Path -Leaf $installer)" | Out-File -FilePath "$installer.sha256" -Encoding ascii -NoNewline
Write-Host "Built $installer"
Write-Host "SHA-256 $hash"
