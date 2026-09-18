param([Parameter(Mandatory)][string]$PackageDirectory)
$ErrorActionPreference='Stop'
$env:DJI4G_DRIVER_ROOT=(Resolve-Path -LiteralPath $PackageDirectory).Path
$env:DJI4G_DRIVER_MODE='plan'
$env:DJI4G_TEST_SCRIPT=Join-Path $PSScriptRoot 'local-driver-install.ps1'
$testCommand=@'
function Get-CimInstance {
 param($ClassName,$Filter,$OperationTimeoutSec)
 switch ($env:DJI4G_TEST_SCENARIO) {
  'healthy' { [pscustomobject]@{ConfigManagerErrorCode=0;PNPDeviceID='USB\VID_2CA3&PID_4006&MI_02\TEST';HardwareID=@('USB\VID_2CA3&PID_4006&MI_02')} }
  'matching' { [pscustomobject]@{ConfigManagerErrorCode=28;PNPDeviceID='USB\VID_2CA3&PID_4006&MI_02\TEST';HardwareID=@('USB\VID_2CA3&PID_4006&MI_02')} }
  'unmatched' { [pscustomobject]@{ConfigManagerErrorCode=28;PNPDeviceID='USB\VID_2CA3&PID_4006&MI_99\TEST';HardwareID=@('USB\VID_2CA3&PID_4006&MI_99')} }
  'disconnected' { }
  default { throw 'Unexpected fixture' }
 }
}
& $env:DJI4G_TEST_SCRIPT
'@
foreach ($case in @(
 @{name='healthy';code=0;expected='Nothing installed'},
 @{name='matching';code=0;expected='qcser.inf'},
 @{name='unmatched';code=1;expected='no unique matching driver'},
 @{name='disconnected';code=1;expected='No supported DJI'}
)) {
 $env:DJI4G_TEST_SCENARIO=$case.name
 $output=& (Join-Path $PSHOME 'pwsh.exe') -NoProfile -Command $testCommand
 if ($LASTEXITCODE -ne $case.code -or ($output -join "`n") -notmatch [regex]::Escape($case.expected)) { throw "Failed fixture: $($case.name): $output" }
 Write-Output "PASS: $($case.name) (read-only synthetic interface)"
}
