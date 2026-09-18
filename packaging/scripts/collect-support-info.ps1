# Read-only support collection. One bounded child process per section; never installs drivers.
$ErrorActionPreference='Stop'
[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false)
try {
 $data = switch ($env:DJI4G_DIAG_SECTION) {
  'system' {
   $os=Get-ItemProperty 'HKLM:/SOFTWARE/Microsoft/Windows NT/CurrentVersion'
   [pscustomobject]@{
    ProductName=$os.ProductName;DisplayVersion=$os.DisplayVersion;Build=$os.CurrentBuild;UBR=$os.UBR
    Is64BitOS=[Environment]::Is64BitOperatingSystem;Is64BitProcess=[Environment]::Is64BitProcess
    PowerShell=$PSVersionTable.PSVersion.ToString()
    Processes=@(Get-Process -ErrorAction Stop | Where-Object { $_.ProcessName -in @('dji4g-panel','dji4g-driver-setup','dji4g-portable','dji4g-helper') } | Select-Object ProcessName,Id,StartTime,Path)
   }
  }
  'usb-and-problem-devices' {
   $all=@(Get-CimInstance Win32_PnPEntity -Filter "PNPDeviceID LIKE 'USB%VID_2CA3%' OR Name LIKE '%Quectel%' OR Name LIKE '%Baiwang%' OR PNPClass='Ports' OR PNPClass='Modem' OR ConfigManagerErrorCode <> 0" -OperationTimeoutSec 20)
   $selected=@($all | Where-Object {
    $_.PNPDeviceID -like 'USB\VID_2CA3*' -or $_.Name -match 'DJI|Quectel|Baiwang|大疆' -or
    $_.PNPClass -in @('Ports','Modem') -or ($_.Present -and $_.ConfigManagerErrorCode -ne 0)
   })
   [pscustomobject]@{TotalMatches=$selected.Count;Limit=256;Devices=@($selected | Select-Object -First 256 Name,PNPClass,PNPDeviceID,HardwareID,CompatibleID,Service,Manufacturer,Status,Present,ConfigManagerErrorCode,ConfigManagerUserConfig)}
  }
  'installed-drivers' {
   @(Get-CimInstance Win32_PnPSignedDriver -Filter "DeviceID LIKE 'USB%VID_2CA3%' OR DeviceName LIKE '%Quectel%' OR DeviceName LIKE '%Baiwang%' OR DeviceClass='PORTS' OR DeviceClass='MODEM'" -OperationTimeoutSec 20 | Where-Object {
    $_.DeviceID -like 'USB\VID_2CA3*' -or $_.DeviceName -match 'DJI|Quectel|Baiwang|大疆' -or $_.DeviceClass -in @('PORTS','MODEM')
   } | Select-Object -First 256 DeviceName,DeviceID,InfName,DriverVersion,DriverDate,DriverProviderName,IsSigned,Signer,DeviceClass)
  }
  'serial-ports' {
   [pscustomobject]@{
    Registry=if(Test-Path 'HKLM:/HARDWARE/DEVICEMAP/SERIALCOMM') { Get-ItemProperty 'HKLM:/HARDWARE/DEVICEMAP/SERIALCOMM' | Select-Object * -ExcludeProperty PSPath,PSParentPath,PSChildName,PSDrive,PSProvider } else { 'SERIALCOMM key absent' }
    Ports=@(Get-CimInstance Win32_SerialPort -OperationTimeoutSec 5 | Select-Object DeviceID,PNPDeviceID,Name,Status,ConfigManagerErrorCode)
   }
  }
  'network-adapters' {
   @(Get-NetAdapter -IncludeHidden | Select-Object -First 128 Name,InterfaceDescription,InterfaceIndex,InterfaceGuid,Status,LinkSpeed,MediaConnectionState,DriverInformation,DriverFileName,DriverVersion,PnPDeviceID)
  }
  'ip-dns-routes' {
   [pscustomobject]@{
    Addresses=@(Get-NetIPAddress | Select-Object -First 256 InterfaceIndex,InterfaceAlias,AddressFamily,IPAddress,PrefixLength,AddressState,PrefixOrigin)
    Dns=@(Get-DnsClientServerAddress | Select-Object -First 128 InterfaceIndex,InterfaceAlias,AddressFamily,ServerAddresses)
    DefaultRoutes=@(Get-NetRoute | Where-Object { $_.DestinationPrefix -in @('0.0.0.0/0','::/0') } | Select-Object -First 128 InterfaceIndex,InterfaceAlias,DestinationPrefix,NextHop,RouteMetric,State)
   }
  }
  'security-products' {
   @(Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntivirusProduct -OperationTimeoutSec 5 | Select-Object displayName,productState)
  }
  'driver-install-events' {
   $log=Join-Path ([Environment]::GetFolderPath('Windows')) 'INF/setupapi.dev.log'
   if (!(Test-Path -LiteralPath $log)) { throw 'setupapi.dev.log is absent or inaccessible' }
   $tail=@(Get-Content -LiteralPath $log -Tail 3000)
   $matches=@($tail | Select-String -Pattern 'VID_2CA3|qcser|qcmdm|qcfilter|qcusb|Baiwang|dji4g' -Context 6,14)
   [pscustomobject]@{Source='setupapi.dev.log';Scope='last 3000 lines; matching excerpts only';Matches=$matches.Count;Excerpts=@($matches | Select-Object -Last 40 | ForEach-Object { @($_.Context.PreContext)+$_.Line+@($_.Context.PostContext) })}
  }
  'bundle-integrity' {
   $root=$env:DJI4G_DIAG_APP_ROOT
   foreach ($name in @('dji4g-panel.exe','dji4g-helper.exe','dji4g-driver-setup.exe','portable-manifest.json','drivers/qcser.inf','drivers/qcser.cat','drivers/qcmdm.inf','drivers/qcmdm.cat','drivers/qcfilter.inf','drivers/qcfilter.cat','drivers/serial/amd64/qcusbser.sys','drivers/filter/amd64/qcusbfilter.sys')) {
    $path=Join-Path $root $name
    if (Test-Path -LiteralPath $path -PathType Leaf) {
     $item=Get-Item -LiteralPath $path
     [pscustomobject]@{Name=$name;Exists=$true;Bytes=$item.Length;Sha256=(Get-FileHash -LiteralPath $path).Hash;Signature=if ($item.Extension -in @('.exe','.cat','.sys')) { (Get-AuthenticodeSignature -LiteralPath $path).Status.ToString() } else { 'not-applicable' }}
    } else { [pscustomobject]@{Name=$name;Exists=$false} }
   }
  }
  default { throw 'Unknown diagnostic section' }
 }
 [pscustomobject]@{Section=$env:DJI4G_DIAG_SECTION;CapturedUtc=[DateTime]::UtcNow.ToString('o');Data=@($data)} | ConvertTo-Json -Depth 8
} catch {
 [Console]::Error.WriteLine(($_ | Out-String))
 exit 1
}
