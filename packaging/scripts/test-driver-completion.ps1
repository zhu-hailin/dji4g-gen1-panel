$ErrorActionPreference='Stop'
# Load only the pure result classifier via the parser. Never execute installation code.
$tokens=$null
$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'local-driver-install.ps1'),[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw 'Installer script parse failure' }
$function=$ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Assert-DriverCompletion'},$true)
if (!$function) { throw 'Missing result classifier' }
. ([scriptblock]::Create($function.Extent.Text))
foreach ($case in @(
 @{name='healthy';codes=@(0,0);restart=$false;fails=$false},
 @{name='missing-driver-remains';codes=@(0,28);restart=$false;fails=$true},
 @{name='other-interface-error';codes=@(10);restart=$false;fails=$true},
 @{name='unknown-status';codes=@($null);restart=$false;fails=$true},
 @{name='disconnected';codes=@();restart=$false;fails=$true},
 @{name='restart-required';codes=@(28);restart=$true;fails=$false}
)) {
 $devices=@($case.codes | ForEach-Object { [pscustomobject]@{ConfigManagerErrorCode=$_} })
 $failed=$false
 try { $null=Assert-DriverCompletion -Devices $devices -NeedsRestart $case.restart } catch { $failed=$true }
 if ($failed -ne $case.fails) { throw "FAIL: $($case.name): failure=$failed, expected=$($case.fails)" }
 Write-Output "PASS: $($case.name)"
}
