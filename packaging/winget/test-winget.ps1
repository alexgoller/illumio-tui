# Install, upgrade and uninstall illumio-tui with winget from local manifests
# (CI, Windows). The manifests are rendered by render.sh; their InstallerUrls
# must be reachable.
#   test-winget.ps1 -Manifest dir -Version 1.2.3 -MsiVersion 1.2.3 `
#     -UpgradeManifest dir2 -UpgradeVersion 1.2.4
# -Version is the package version (what --version prints), -MsiVersion the
# MSI ProductVersion it maps to (1.2.3-rc1 -> 1.2.3). The upgrade package is
# the same binary in an MSI with a higher version.
# With a local manifest, winget matches the installed package only by that
# manifest's installer ProductCode; the winget-pkgs source also knows the
# ProductCodes of earlier versions. The upgrade manifest must therefore carry
# the ProductCode of the MSI behind -Manifest (as the source would map it),
# not its own. Fails on the first broken expectation.
param(
    [Parameter(Mandatory)] [string] $Manifest,
    [Parameter(Mandatory)] [string] $Version,
    [Parameter(Mandatory)] [string] $MsiVersion,
    [Parameter(Mandatory)] [string] $UpgradeManifest,
    [Parameter(Mandatory)] [string] $UpgradeVersion
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Must match UpgradeCode in packaging/windows/illumio-tui.wxs.
$UpgradeCode = '{44708F5A-DA8B-483B-BD6A-F5E4B1A34D7F}'
$Dir = Join-Path $env:LOCALAPPDATA 'Programs\illumio-tui'
$Exe = Join-Path $Dir 'illumio-tui.exe'
$Common = @('--accept-package-agreements', '--accept-source-agreements', '--disable-interactivity')

function Invoke-Winget([string[]] $Arguments) {
    Write-Host "winget $($Arguments -join ' ')"
    & winget @Arguments | Write-Host
    if ($LASTEXITCODE -ne 0) { throw "winget $($Arguments[0]) exited with $LASTEXITCODE (0x$('{0:X8}' -f $LASTEXITCODE))" }
}

$Installer = New-Object -ComObject WindowsInstaller.Installer

# Installed products with our UpgradeCode, as product code -> version.
function Get-Related {
    $codes = @($Installer.GetType().InvokeMember('RelatedProducts', 'GetProperty', $null, $Installer, @($UpgradeCode)))
    $out = [ordered]@{}
    foreach ($c in $codes) {
        $out[$c] = $Installer.GetType().InvokeMember('ProductInfo', 'GetProperty', $null, $Installer, @($c, 'VersionString'))
    }
    $out
}

function Assert-Installed([string] $Expected) {
    $related = Get-Related
    $related | Format-Table | Out-String | Write-Host
    if ($related.Count -ne 1) { throw "expected 1 installed product, found $($related.Count)" }
    $v = @($related.Values)[0]
    if ($v -ne $Expected) { throw "installed version $v, expected $Expected" }
    # As a new terminal would see it: PATH from the registry, not this process.
    $env:Path = [Environment]::GetEnvironmentVariable('Path', 'User') + ';' + [Environment]::GetEnvironmentVariable('Path', 'Machine')
    $cmd = Get-Command illumio-tui -ErrorAction SilentlyContinue
    if ($null -eq $cmd) { throw 'illumio-tui is not on the user PATH' }
    if ($cmd.Source -ne $Exe) { throw "illumio-tui resolves to $($cmd.Source), expected $Exe" }
    $out = illumio-tui --version
    if ($LASTEXITCODE -ne 0) { throw "illumio-tui --version exited with $LASTEXITCODE" }
    Write-Host "illumio-tui --version: $out"
    # Both MSIs carry the same binary, which reports the first version.
    if ("$out" -notmatch [regex]::Escape($Version)) { throw "unexpected --version output: $out" }
}

winget --info | Write-Host
Invoke-Winget @('settings', '--enable', 'LocalManifestFiles')
Invoke-Winget @('validate', '--manifest', $Manifest)
Invoke-Winget @('validate', '--manifest', $UpgradeManifest)

if ((Get-Related).Count -ne 0) { throw 'illumio-tui is already installed on this runner' }

Write-Host "== winget install $Version"
Invoke-Winget (@('install', '--manifest', $Manifest, '--silent') + $Common)
Assert-Installed $MsiVersion

Write-Host "== winget upgrade to $UpgradeVersion"
Invoke-Winget (@('upgrade', '--manifest', $UpgradeManifest, '--silent') + $Common)
Assert-Installed $UpgradeVersion

Write-Host '== winget uninstall'
$code = @((Get-Related).Keys)[0]
Invoke-Winget @('uninstall', '--product-code', $code, '--silent', '--accept-source-agreements', '--disable-interactivity')
if ((Get-Related).Count -ne 0) { throw 'product still installed after uninstall' }
if (Test-Path $Dir) { throw "$Dir still exists after uninstall" }
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (@("$userPath" -split ';' | Where-Object { $_.TrimEnd('\') -ieq $Dir }).Count -ne 0) { throw "user PATH still holds $Dir" }
Write-Host 'winget validate, install, upgrade and uninstall: ok'
