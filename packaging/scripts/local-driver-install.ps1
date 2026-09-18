# Embedded in dji4g-driver-setup.exe. No downloads, INF editing or forced binding.
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
function Assert-DriverCompletion {
 param([object[]]$Devices, [bool]$NeedsRestart)
 if (!$Devices.Count) { throw 'Device disconnected; installation result requires a fresh check.' }
 $remaining=@($Devices | Where-Object { $_.ConfigManagerErrorCode -ne 0 })
 Write-Output ("Driver staging finished. Interfaces not yet OK: {0}" -f $remaining.Count)
 if ($NeedsRestart) {
  Write-Output '需要重启：Windows 已暂存驱动，当前不能确认接口安装成功。请重启后重新检查。'
 } elseif ($remaining.Count) {
  throw 'DRIVER_NOT_READY: 接口仍异常，不能判定安装成功。请保留日志并检查设备管理器；未强制绑定驱动。'
 } else {
  Write-Output '驱动接口状态正常；仍需单独验证 AT 通信和网络。'
 }
}
$root = $env:DJI4G_DRIVER_ROOT
$install = $env:DJI4G_DRIVER_MODE -eq 'install'
$planOnly = $env:DJI4G_DRIVER_MODE -eq 'plan'
$expected = @{
 'qcfilter.cat'='DC7D84F79DEA6258F396D4F8871F5F86FF581A0E39980AC22C13AE2B2B8D0054'
 'qcfilter.inf'='9F38F6AC063C2D2B7AADD948987D89183916E444852150C1590F2DFD6CE26598'
 'qcmdm.cat'='2A49C3484DAC3B605FAE720D3CA661222A6E70C6EDDF28C39F93CE0980C535A0'
 'qcmdm.inf'='44201E52A538B1EE7067F1E9E55364C413158D897B10DCA12C461A18FC193A95'
 'qcser.cat'='171A8804E5295D942201393C44389DABDE96F97EEADD9931629AEB8FB4FB2043'
 'qcser.inf'='08F4300656AFBECAB57B09EB55FA5C336F2BDCAD1B579C38DE81B4256BD1736A'
 'filter/amd64/qcusbfilter.sys'='997507BDC5D5203B994C0064CBAE66E9AD43C11E7FC27F13B1868F84327D2B3C'
 'serial/amd64/qcusbser.sys'='FE77A74CBB798E0B6E49437CC20D0C6A6EDF1D164CE1CFB5E3157DB75943B87D'
}
$locks = [Collections.Generic.List[IO.FileStream]]::new()
try {
 if (![Environment]::Is64BitProcess -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { throw 'This package requires Windows x64.' }
 foreach ($name in $expected.Keys) {
  $path=Join-Path $root $name
  # Deny writes/deletion while hashes are checked and PnPUtil reads the files.
  $locks.Add([IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read))
  if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $expected[$name]) { throw "HASH_MISMATCH: $name" }
 }
 foreach ($name in @('qcfilter.cat','qcser.cat','qcmdm.cat')) {
  if ((Get-AuthenticodeSignature -LiteralPath (Join-Path $root $name)).Status -ne 'Valid') { throw "CATALOG_UNTRUSTED: $name" }
 }
 Write-Output 'Package hashes and catalog signatures verified.'
 if (!$install -and !$planOnly) { exit 0 }
 if ($install) {
 $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
 if (!$principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Administrator permission is required.' }
 Write-Output '大疆一代模块离线驱动安装。正常工作的接口会跳过。'
 Write-Output '请先关闭大疆 4G 面板。安装期间请勿发送短信或拔插模块。'
 if (Get-Process -Name 'dji4g-panel' -ErrorAction SilentlyContinue) { throw 'Close DJI 4G Panel and retry.' }
 }
 $deviceFilter = "PNPDeviceID LIKE 'USB\\VID_2CA3&PID_4006%' AND Present = TRUE"
 $devices = @(Get-CimInstance Win32_PnPEntity -Filter $deviceFilter -OperationTimeoutSec 20)
 if (!$devices.Count) { throw 'No supported DJI Gen1 USB device connected.' }
 $missing = @($devices | Where-Object { $_.ConfigManagerErrorCode -eq 28 })
 if (!$missing.Count) { Write-Output 'No missing-driver interfaces (Code 28). Nothing installed.'; exit 0 }
 $plans = @()
 foreach ($device in $missing) {
  $hardwareIds = @($device.HardwareID)
  $matchesForDevice = @()
  foreach ($inf in @('qcfilter.inf','qcser.inf','qcmdm.inf')) {
   $section=''
   foreach ($line in (Get-Content -LiteralPath (Join-Path $root $inf))) {
    $active=($line -split ';',2)[0].Trim()
    if ($active -match '^\[([^\]]+)\]$') { $section=$Matches[1]; continue }
    if ($section -notmatch '(?i)\.NTamd64(?:\.|$)' -or $active -notmatch '=') { continue }
    $ids=@((($active -split '=',2)[1] -split ',' | Select-Object -Skip 1) | ForEach-Object { $_.Trim().Trim('"') })
    if (@($ids | Where-Object { $hardwareIds -contains $_ }).Count) { $matchesForDevice += $inf; break }
   }
  }
  if ($matchesForDevice.Count -ne 1) { throw 'A missing interface has no unique matching driver. Nothing installed; no forced binding.' }
  $plans += [pscustomobject]@{inf=$matchesForDevice[0]; instance=$device.PNPDeviceID}
 }
 if ($planOnly) {
  [pscustomobject]@{mode='read-only plan'; missingInterfaces=$missing.Count; driverPackages=@($plans.inf | Select-Object -Unique); installationPerformed=$false} | ConvertTo-Json
  exit 0
 }
 $pnputil=Join-Path ([Environment]::SystemDirectory) 'pnputil.exe'
 $needsRestart=$false
 foreach ($inf in @($plans.inf | Select-Object -Unique)) {
  # Stage only: /install would also update unrelated matching devices.
  & $pnputil /add-driver (Join-Path $root $inf)
  if ($LASTEXITCODE -eq 3010) { $needsRestart=$true }
  elseif ($LASTEXITCODE -ne 0) { throw "DRIVER_STAGE_FAILED: $LASTEXITCODE" }
 }
 foreach ($plan in $plans) {
  & $pnputil /scan-devices /instanceid $plan.instance
  if ($LASTEXITCODE -eq 3010) { $needsRestart=$true }
  elseif ($LASTEXITCODE -ne 0) { throw "DEVICE_SCAN_FAILED: $LASTEXITCODE" }
 }
 if ($needsRestart) { Write-Output 'Windows requested a restart. Restart before verifying device operation.' }
 $after=@(Get-CimInstance Win32_PnPEntity -Filter $deviceFilter -OperationTimeoutSec 20)
 Assert-DriverCompletion -Devices $after -NeedsRestart $needsRestart
 Write-Output 'Reopen DJI 4G Panel > Repair > First connection checks. Verify AT and network separately.'
 Write-Output 'PnP status alone does not prove SMS or Internet operation. New child interfaces may require running setup again.'
} catch { Write-Output ("ERROR: " + $_.Exception.Message); exit 1 }
finally { foreach ($handle in $locks) { $handle.Dispose() } }
