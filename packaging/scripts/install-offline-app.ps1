$ErrorActionPreference='Stop'
[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false)
try {
 $zip=$env:DJI4G_SETUP_PAYLOAD
 $hash=$env:DJI4G_SETUP_HASH
 if ($hash -notmatch '^[a-fA-F0-9]{64}$' -or (Get-FileHash -LiteralPath $zip).Hash -ne $hash) { throw '安装资源校验失败' }
 $base=[Environment]::GetFolderPath('LocalApplicationData')
 $root=Join-Path $base ('Dji4GPanel/versions/'+$hash.Substring(0,16))
 Add-Type -AssemblyName System.IO.Compression.FileSystem
 if (!(Test-Path -LiteralPath $root)) {
  [IO.Directory]::CreateDirectory($root) | Out-Null
  [IO.Compression.ZipFile]::ExtractToDirectory($zip,$root)
 }
 $manifest=Get-Content -LiteralPath (Join-Path $root 'portable-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
 foreach ($entry in $manifest) {
  if ((Get-FileHash -LiteralPath (Join-Path $root $entry.name)).Hash -ne $entry.sha256) { throw ('已安装文件校验失败：'+$entry.name) }
 }
 foreach ($relative in @('dji4g-panel.exe','dji4g-driver-setup.exe','drivers/qcser.inf','drivers/qcser.cat')) {
  if (!(Test-Path -LiteralPath (Join-Path $root $relative) -PathType Leaf)) { throw ('缺少安装文件：'+$relative) }
 }
 $shell=New-Object -ComObject WScript.Shell
 $desktop=[Environment]::GetFolderPath('DesktopDirectory')
 foreach ($link in @(@{name='大疆4G面板.lnk';exe='dji4g-panel.exe'},@{name='安装大疆4G驱动.lnk';exe='dji4g-driver-setup.exe'})) {
  $shortcut=$shell.CreateShortcut((Join-Path $desktop $link.name))
  $shortcut.TargetPath=Join-Path $root $link.exe
  $shortcut.WorkingDirectory=$root
  $shortcut.Save()
 }
 Write-Output $root
} catch { Write-Output ('安装失败：'+$_.Exception.Message); exit 1 }
