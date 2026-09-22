$ErrorActionPreference='Stop'
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'local-driver-install.ps1'),[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Installer script parse failure' }
$function=$ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Assert-DriverCompletion'},$true)
. ([scriptblock]::Create($function.Extent.Text))
foreach ($case in @(
 @{name='healthy';codes=@(0,0);restart=$false;result='ready';fails=$false},
 @{name='missing-driver-remains';codes=@(0,28);restart=$false;result='not-ready';fails=$true},
 @{name='unknown-status';codes=@($null);restart=$false;result='not-ready';fails=$true},
 @{name='disconnected';codes=@();restart=$false;result='disconnected';fails=$true},
 @{name='restart-required';codes=@(28);restart=$true;result='restart-required';fails=$false},
 @{name='restart-after-disconnect';codes=@();restart=$true;result='restart-required';fails=$false}
)) {
 $devices=@($case.codes | ForEach-Object { [pscustomobject]@{ConfigManagerErrorCode=$_} })
 $failed=$false; $result=''
 try { $result=@(Assert-DriverCompletion -Devices $devices -NeedsRestart $case.restart) -join "`n" } catch { $failed=$true; $result=$_.Exception.Message }
 if ($failed -ne $case.fails -or $result -notmatch [regex]::Escape($case.result)) { throw "FAIL: $($case.name): result=$result failed=$failed" }
 Write-Output "PASS: $($case.name)"
}
