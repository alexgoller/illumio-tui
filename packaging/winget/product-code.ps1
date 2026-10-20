# Print an MSI's ProductCode ({GUID}), read with the Windows Installer API.
#   product-code.ps1 illumio-tui.msi
param([Parameter(Mandatory)] [string] $Msi)
$ErrorActionPreference = 'Stop'
$installer = New-Object -ComObject WindowsInstaller.Installer
$db = $installer.GetType().InvokeMember('OpenDatabase', 'InvokeMethod', $null, $installer, @((Resolve-Path $Msi).Path, 0))
$view = $db.GetType().InvokeMember('OpenView', 'InvokeMethod', $null, $db, @("SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = 'ProductCode'"))
$view.GetType().InvokeMember('Execute', 'InvokeMethod', $null, $view, $null) | Out-Null
$record = $view.GetType().InvokeMember('Fetch', 'InvokeMethod', $null, $view, $null)
if ($null -eq $record) { throw "no ProductCode in $Msi" }
$code = $record.GetType().InvokeMember('StringData', 'GetProperty', $null, $record, @(1))
$view.GetType().InvokeMember('Close', 'InvokeMethod', $null, $view, $null) | Out-Null
if ($code -notmatch '^\{[0-9A-F]{8}(-[0-9A-F]{4}){3}-[0-9A-F]{12}\}$') { throw "unexpected ProductCode '$code' in $Msi" }
$code
