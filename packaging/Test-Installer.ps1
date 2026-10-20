<#
.SYNOPSIS
  Smoke-tests the installer on a clean Windows machine (CI runs it on windows-latest).

.DESCRIPTION
  Per-user silent install -> files and Start menu shortcut exist -> the app starts and is still
  running after a few seconds -> reinstall over itself -> a downgrade is refused -> uninstall stops
  the app and leaves no files or shortcut.

  Changes the machine it runs on; meant for throwaway CI runners and test VMs.

.EXAMPLE
  .\packaging\Test-Installer.ps1 -Installer artifacts\dbclient-Setup-0.1.0.exe
#>
param(
    [Parameter(Mandatory)][string]$Installer,
    [string]$LogDir = (Join-Path ([System.IO.Path]::GetTempPath()) 'dbclient-installer-logs')
)

$ErrorActionPreference = 'Stop'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\dbclient'
$exe = Join-Path $installDir 'dbclient.exe'
$shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\dbclient.lnk'
New-Item $LogDir -ItemType Directory -Force | Out-Null
$failures = [System.Collections.Generic.List[string]]::new()

function Check([bool]$condition, [string]$what) {
    if ($condition) { Write-Host "  ok   $what" }
    else { Write-Host "  FAIL $what" -ForegroundColor Red; $failures.Add($what) }
}

function Invoke-Setup([string[]]$extra = @()) {
    $log = Join-Path $LogDir ("setup-{0}.log" -f [DateTime]::Now.Ticks)
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', "/LOG=`"$log`"") + $extra
    return (Start-Process -FilePath $Installer -ArgumentList $arguments -Wait -PassThru).ExitCode
}

Write-Host "Install (per user)"
Check ((Invoke-Setup) -eq 0) "installer exits 0"
Check (Test-Path $exe) "executable installed to $installDir"
Check (Test-Path $shortcut) "Start menu shortcut created"
foreach ($f in "LICENSE.txt", "THIRD_PARTY_NOTICES.md", "THIRD_PARTY_LICENSES.txt") {
    Check (Test-Path (Join-Path $installDir $f)) "$f installed"
}

Write-Host "Run"
$p = Start-Process -FilePath $exe -PassThru
Start-Sleep -Seconds 8
Check (-not $p.HasExited) "app is still running after 8 s (exit code: $(if ($p.HasExited) { $p.ExitCode } else { 'n/a' }))"
Check (Test-Path (Join-Path $env:USERPROFILE '.dbclient\log.txt')) "log file was written to ~\.dbclient"

Write-Host "Reinstall over itself (app running)"
Check ((Invoke-Setup) -eq 0) "reinstall exits 0"
Check (Test-Path $exe) "executable still present"

Write-Host "Downgrade is refused"
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{7D2B5E64-3A9F-4C1B-9E8D-5F6A1C2B3D4E}_is1'
$realVersion = (Get-ItemProperty $uninstallKey).DisplayVersion
Set-ItemProperty $uninstallKey -Name DisplayVersion -Value '999.0.0'
Check ((Invoke-Setup) -ne 0) "installing over a newer version fails"
Set-ItemProperty $uninstallKey -Name DisplayVersion -Value $realVersion

Write-Host "Uninstall (with the app running)"
Start-Process -FilePath $exe | Out-Null
Start-Sleep -Seconds 3
$uninstaller = Join-Path $installDir 'unins000.exe'
Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait | Out-Null
for ($i = 0; $i -lt 120 -and (Test-Path $uninstaller); $i++) { Start-Sleep -Milliseconds 500 }
Check (-not (Get-Process dbclient -ErrorAction SilentlyContinue)) "app stopped"
Check (-not (Test-Path $exe)) "executable removed"
Check (-not (Test-Path $shortcut)) "Start menu shortcut removed"

if ($failures.Count) {
    throw "$($failures.Count) installer check(s) failed: $($failures -join '; ')"
}
Write-Host "All installer checks passed."
