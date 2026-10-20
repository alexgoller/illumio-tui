# Install, upgrade and uninstall test for the illumio-tui MSI (CI, Windows).
#   test-msi.ps1 -Msi a.msi -Version 1.2.3 -UpgradeMsi b.msi -UpgradeVersion 1.2.4
# -UpgradeMsi is the same binary packaged with a higher version. Fails on the
# first broken expectation; msiexec logs land next to the MSIs.
param(
    [Parameter(Mandatory)] [string] $Msi,
    [Parameter(Mandatory)] [string] $Version,
    [Parameter(Mandatory)] [string] $UpgradeMsi,
    [Parameter(Mandatory)] [string] $UpgradeVersion
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

# Must match UpgradeCode in illumio-tui.wxs.
$UpgradeCode = '{44708F5A-DA8B-483B-BD6A-F5E4B1A34D7F}'
$Dir = Join-Path $env:LOCALAPPDATA 'Programs\illumio-tui'
$Exe = Join-Path $Dir 'illumio-tui.exe'

function Invoke-Msiexec([string[]] $Arguments, [string] $Log) {
    $all = $Arguments + @('/qn', '/l*v', "`"$Log`"")
    Write-Host "msiexec $($all -join ' ')"
    $p = Start-Process msiexec.exe -ArgumentList $all -Wait -PassThru
    if ($p.ExitCode -ne 0) {
        Get-Content $Log -Tail 60 | Write-Host
        throw "msiexec exited with $($p.ExitCode)"
    }
}

# Installed products with our UpgradeCode, as product code -> version.
function Get-Related {
    $installer = New-Object -ComObject WindowsInstaller.Installer
    $t = $installer.GetType()
    $codes = @($t.InvokeMember('RelatedProducts', 'GetProperty', $null, $installer, @($UpgradeCode)))
    $out = [ordered]@{}
    foreach ($c in $codes) {
        $out[$c] = $t.InvokeMember('ProductInfo', 'GetProperty', $null, $installer, @($c, 'VersionString'))
    }
    $out
}

# Entries of the user PATH (registry, not this process) equal to the install dir.
function Get-PathHits {
    $path = (Get-ItemProperty -Path 'HKCU:\Environment' -Name Path -ErrorAction SilentlyContinue).Path
    @("$path" -split ';' | Where-Object { $_.TrimEnd('\') -ieq $Dir })
}

function Assert-Installed([string] $Expected) {
    $related = Get-Related
    $related | Format-Table | Out-String | Write-Host
    if ($related.Count -ne 1) { throw "expected 1 installed product, found $($related.Count)" }
    $v = @($related.Values)[0]
    if ($v -ne $Expected) { throw "installed version $v, expected $Expected" }
    if (-not (Test-Path $Exe)) { throw "$Exe missing" }
    $out = & $Exe --version
    if ($LASTEXITCODE -ne 0) { throw "illumio-tui --version exited with $LASTEXITCODE" }
    Write-Host "illumio-tui --version: $out"
    # Both MSIs carry the same binary, which reports the first version.
    if ("$out" -notmatch [regex]::Escape($Version)) { throw "unexpected --version output: $out" }
    $hits = Get-PathHits
    if ($hits.Count -ne 1) { throw "user PATH holds $Dir $($hits.Count) times, expected once" }
    $arp = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' |
        Get-ItemProperty | Where-Object { $_.PSObject.Properties['DisplayName'] -and $_.DisplayName -eq 'illumio-tui' })
    if ($arp.Count -ne 1) { throw "expected 1 Add/Remove Programs entry, found $($arp.Count)" }
    if ($arp[0].Publisher -ne 'Alex Goller' -or $arp[0].DisplayVersion -ne $Expected) {
        throw "ARP entry: publisher '$($arp[0].Publisher)', version '$($arp[0].DisplayVersion)'"
    }
}

if ((Get-Related).Count -ne 0) { throw 'illumio-tui is already installed on this runner' }

$logDir = Split-Path -Parent (Resolve-Path $Msi)

Write-Host "== install $Version"
Invoke-Msiexec @('/i', "`"$((Resolve-Path $Msi).Path)`"") (Join-Path $logDir 'install.log')
Assert-Installed $Version

Write-Host '== PowerShell completion'
$env:Path = "$Dir;$env:Path"
& $Exe completions powershell | Out-String | Invoke-Expression
$line = 'illumio-tui compl'
$offered = @((TabExpansion2 -inputScript $line -cursorColumn $line.Length).CompletionMatches |
    ForEach-Object { $_.CompletionText })
if ($offered -notcontains 'completions') { throw "completing '$line' offered: $($offered -join ', ')" }
Write-Host "completing '$line' offers: $($offered -join ', ')"

Write-Host "== upgrade to $UpgradeVersion"
Invoke-Msiexec @('/i', "`"$((Resolve-Path $UpgradeMsi).Path)`"") (Join-Path $logDir 'upgrade.log')
Assert-Installed $UpgradeVersion

Write-Host '== uninstall'
$code = @((Get-Related).Keys)[0]
Invoke-Msiexec @('/x', $code) (Join-Path $logDir 'uninstall.log')
if ((Get-Related).Count -ne 0) { throw 'product still installed after uninstall' }
if (Test-Path $Dir) { throw "$Dir still exists after uninstall" }
if ((Get-PathHits).Count -ne 0) { throw "user PATH still holds $Dir after uninstall" }
Write-Host 'MSI install, upgrade and uninstall: ok'
