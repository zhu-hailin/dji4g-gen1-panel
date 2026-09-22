$ErrorActionPreference='Stop'
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'local-driver-install.ps1'),[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Installer parse failure' }
# Evaluate only exact matching and missing-interface selection; never invoke PnPUtil.
$loop=$ast.Find({param($node) $node -is [Management.Automation.Language.ForEachStatementAst] -and $node.Variable.VariablePath.UserPath -eq 'device'},$true)
if (!$loop) { throw 'Missing exact-match planner' }
$selection=$ast.Find({param($node) $node -is [Management.Automation.Language.AssignmentStatementAst] -and $node.Left.Extent.Text -eq '$missing'},$true)
$root=Join-Path ([IO.Path]::GetTempPath()) ('driver-plan-fixture-'+[guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
$fixture="[Models.NTamd64]`n%Port%=Install,USB\VID_2CA3&PID_4006&MI_02`n[Models.NTx86]`n%Ignored%=Install,USB\VID_2CA3&PID_4006&MI_04"
foreach ($name in @('qcfilter.inf','qcser.inf','qcmdm.inf')) { Set-Content -LiteralPath (Join-Path $root $name) -Value '' }
Set-Content -LiteralPath (Join-Path $root 'qcser.inf') -Value $fixture
foreach ($case in @(
 @{name='healthy-skipped';interfaces=@(@{id='MI_04';code=0});expected=0;fails=$false},
 @{name='exact-match';interfaces=@(@{id='MI_02';code=28});expected=1;fails=$false},
 @{name='unmatched-mi04';interfaces=@(@{id='MI_04';code=28});expected=0;fails=$true},
 @{name='prefix-not-match';interfaces=@(@{id='MI_020';code=28});expected=0;fails=$true},
 @{name='all-before-write';interfaces=@(@{id='MI_02';code=28},@{id='MI_04';code=28});expected=1;fails=$true}
)) {
 $devices=@($case.interfaces | ForEach-Object { [pscustomobject]@{HardwareID=@('USB\VID_2CA3&PID_4006&'+$_.id);PNPDeviceID='test-instance';ConfigManagerErrorCode=$_.code} })
 . ([scriptblock]::Create($selection.Extent.Text))
 $plans=@(); $failed=$false
 try { $null=. ([scriptblock]::Create($loop.Extent.Text)) } catch {
  $failed=$true
  if ($_.Exception.Message -ne 'unsupported-interface') { throw }
 }
 if ($failed -ne $case.fails -or $plans.Count -ne $case.expected) { throw "FAIL: $($case.name) failed=$failed plans=$($plans.Count)" }
 Write-Output "PASS: $($case.name)"
}
Set-Content -LiteralPath (Join-Path $root 'qcmdm.inf') -Value $fixture
$missing=@([pscustomobject]@{HardwareID=@('USB\VID_2CA3&PID_4006&MI_02');PNPDeviceID='test-instance'})
$failed=$false
try { $null=. ([scriptblock]::Create($loop.Extent.Text)) } catch { $failed=$_.Exception.Message -eq 'unsupported-interface' }
if (!$failed) { throw 'Ambiguous driver candidate was not rejected' }
Write-Output 'PASS: ambiguous-match-rejected'

