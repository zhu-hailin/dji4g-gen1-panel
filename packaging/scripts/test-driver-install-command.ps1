$ErrorActionPreference='Stop'
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'local-driver-install.ps1'),[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Installer parse failure' }
# Execute only the package loop, replacing the OS boundary; never install drivers.
$loop=$ast.Find({param($n) $n -is [Management.Automation.Language.ForEachStatementAst] -and $n.Extent.Text -match '/add-driver'},$true)
if (!$loop) { throw 'Missing driver package loop' }
function Invoke-FakePnpUtil {
 $script:calls.Add(@($args))
 if ($script:resultCodes) { $global:LASTEXITCODE=$script:resultCodes[$script:calls.Count-1] }
 else { $global:LASTEXITCODE=$script:resultCode }
}
# Multiple packages retain reboot requirements and stop immediately on any native failure.
$pnputil='Invoke-FakePnpUtil'
$root=$PSScriptRoot
$plans=@([pscustomobject]@{inf='qcser.inf';instance='fixture-a'},[pscustomobject]@{inf='qcmdm.inf';instance='fixture-b'})
foreach ($case in @(
 @{codes=@(5,0);calls=1;restart=$false;fails=$true},
 @{codes=@(3010,0);calls=2;restart=$true;fails=$false},
 @{codes=@(0,5);calls=2;restart=$false;fails=$true}
)) {
 $script:calls=[Collections.Generic.List[object]]::new()
 $script:resultCodes=$case.codes; $needsRestart=$false; $failed=$false
 try { . ([scriptblock]::Create($loop.Extent.Text)) } catch { $failed=$true }
 if ($calls.Count -ne $case.calls -or $needsRestart -ne $case.restart -or $failed -ne $case.fails) { throw 'Wrong multi-package completion result' }
 Write-Output ('PASS: package sequence '+($case.codes -join ','))
}
$scan=$ast.Find({param($n) $n -is [Management.Automation.Language.ForEachStatementAst] -and $n.Extent.Text -match '/scan-devices'},$true)
foreach ($case in @(
 @{codes=@(0,0);calls=2;restart=$false;fails=$false},
 @{codes=@(3010,0);calls=2;restart=$true;fails=$false},
 @{codes=@(5,0);calls=1;restart=$false;fails=$true}
)) {
 $script:calls=[Collections.Generic.List[object]]::new()
 $script:resultCodes=$case.codes; $needsRestart=$false; $failed=$false
 try { . ([scriptblock]::Create($scan.Extent.Text)) } catch { $failed=$true }
 if ($calls.Count -ne $case.calls -or $needsRestart -ne $case.restart -or $failed -ne $case.fails) { throw 'Wrong device scan completion result' }
 if ($calls[0] -notcontains '/instanceid' -or $calls[0] -notcontains 'fixture-a') { throw 'Scan must remain restricted to the planned device' }
 Write-Output ('PASS: scan sequence '+($case.codes -join ','))
}
$pnputil='Invoke-FakePnpUtil'
$root=$PSScriptRoot
$plans=@([pscustomobject]@{inf='qcser.inf'},[pscustomobject]@{inf='qcser.inf'})
$script:resultCodes=$null
foreach ($code in @(0,3010,5)) {
 $script:calls=[Collections.Generic.List[object]]::new()
 $script:resultCode=$code; $needsRestart=$false; $failed=$false
 try { . ([scriptblock]::Create($loop.Extent.Text)) } catch { $failed=$true }
 if ($calls.Count -ne 1 -or $calls[0] -notcontains '/install') { throw 'FAIL: package must be installed on matching devices, not only staged' }
 if ($failed -ne ($code -eq 5)) { throw "Wrong failure outcome: $code" }
 if ($needsRestart -ne ($code -eq 3010)) { throw "Wrong restart outcome: $code" }
 Write-Output "PASS: install command exit $code"
}
# Run real production catch logic; capture its exit instead of terminating the test process.
# Native calls remain mocked; no file hashes, device enumeration or installation are executed.
$tryAst=$ast.Find({param($n) $n -is [Management.Automation.Language.TryStatementAst] -and $n.CatchClauses.Count -gt 0},$true)
$catchBody=$tryAst.CatchClauses[0].Body.Extent.Text.Replace('exit 1','$script:capturedExit = 1')
foreach ($phase in @('later-package','device-scan','package-then-scan')) {
 $script:calls=[Collections.Generic.List[object]]::new()
 $script:resultCodes=if ($phase -eq 'package-then-scan') {@(3010,0,5)} else {@(3010,5)}
 $script:capturedExit=$null
 $plans=@([pscustomobject]@{inf='qcser.inf';instance='fixture-a'},[pscustomobject]@{inf='qcmdm.inf';instance='fixture-b'})
 $body=if ($phase -eq 'later-package') {$loop.Extent.Text} elseif ($phase -eq 'device-scan') {$scan.Extent.Text} else {$loop.Extent.Text+[Environment]::NewLine+$scan.Extent.Text}
 $fixture='$needsRestart=$false; $failureResult="failed"; try {'+$body+'} catch '+$catchBody
 $output=@(& ([scriptblock]::Create($fixture))) -join [Environment]::NewLine
 if ($output -notmatch '(?m)^DRIVER_SETUP_RESULT=restart-required-after-failure\r?$') {throw "Lost restart requirement after $phase failure: $output"}
 if ($output -match '(?m)^DRIVER_SETUP_RESULT=ready\r?$') {throw 'Partial failure must not be successful readiness'}
 if ($script:capturedExit -ne 1) {throw 'Partial failure must keep the failed process status'}
 Write-Output "PASS: 3010 then $phase failure preserves partial failure and restart"
}
# The fake native process deliberately leaves 5 in LASTEXITCODE in the last case.
# GitHub's pwsh wrapper uses that value as the job result; no real command failed.
$global:LASTEXITCODE=0
