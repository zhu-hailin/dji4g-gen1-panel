$ErrorActionPreference='Stop'
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'local-driver-install.ps1'),[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Installer parse failure' }
# Execute only the package loop, replacing the OS boundary; never install drivers.
$loop=$ast.Find({param($n) $n -is [Management.Automation.Language.ForEachStatementAst] -and $n.Extent.Text -match '/add-driver'},$true)
if (!$loop) { throw 'Missing driver package loop' }
function Invoke-FakePnpUtil {
 $script:calls.Add(@($args))
 $global:LASTEXITCODE=$script:resultCode
}
$pnputil='Invoke-FakePnpUtil'
$root=$PSScriptRoot
$plans=@([pscustomobject]@{inf='qcser.inf'},[pscustomobject]@{inf='qcser.inf'})
foreach ($code in @(0,3010,5)) {
 $script:calls=[Collections.Generic.List[object]]::new()
 $script:resultCode=$code; $needsRestart=$false; $failed=$false
 try { . ([scriptblock]::Create($loop.Extent.Text)) } catch { $failed=$true }
 if ($calls.Count -ne 1 -or $calls[0] -notcontains '/install') { throw 'FAIL: package must be installed on matching devices, not only staged' }
 if ($failed -ne ($code -eq 5)) { throw "Wrong failure outcome: $code" }
 if ($needsRestart -ne ($code -eq 3010)) { throw "Wrong restart outcome: $code" }
 Write-Output "PASS: install command exit $code"
}
