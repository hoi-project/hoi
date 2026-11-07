#Requires -Version 5.1
<#
.SYNOPSIS
The hoi installer for Windows: the desktop app, the companion, or a node that runs in the
background (a home node, which is a public relay too; a public node; a public node with a
registry). macOS and Linux: install.sh. Run it again to update or to switch roles; the data
stays. Nothing it installs names a node or a registry to start from: nodes find each other.

.DESCRIPTION
  irm https://hoi.sh/install.ps1 | iex                                       asks what to install
  & ([scriptblock]::Create((irm https://hoi.sh/install.ps1))) -Role desktop
  & ([scriptblock]::Create((irm https://hoi.sh/install.ps1))) -Role home -Name "lena's pc"
  .\deploy\install\install.ps1 -Role public -From dist\release\v0.2.0         a release built on a computer

What (-Role):
  desktop     the hoi app: it opens as the companion in the tray, the full app a click away
  companion   the same app, started in the tray whenever you log in
  home        a home node for your phone and browser that is a public relay too, started when
              you log in: it carries open communities for whoever asks, tells whoever connects
              about the other public nodes it reached, and has a link to share
  relay       the same as home (its name from before every home node was a public relay)
  private     a home node for your own devices and your communities' members only
  public      a public node: no key, carries open communities, passes on other public nodes
  registry    a public node with a registry (the public-name API), behind your TLS proxy
  uninstall   stop and remove the node this script set up (the data stays)

From where (default: the latest release in https://hoi.sh/releases/manifest.json):
  -Manifest URL, -Release TAG (with the GitHub CLI; works for a private repository),
  -Repo OWNER/NAME, -From DIR (a release folder from scripts/release-local.mjs),
  -Binary PATH (a hoi-node.exe or the release archive holding it), -PubKey KEY (minisign).

Nodes: -Node LINK (a public node to look at first, repeatable; optional), -Index URL (a registry,
repeatable; optional, none is needed), -Name, -DailyCapGb N, -HoldPiecesGb N (public nodes: keep up
to N GB of encrypted pieces and private communities' mailboxes for others; it can read none of
it, and its notice says so), -RtcAdvertise IP:PORT, -NoPortMapping,
-RegistryUrl URL, -RegistryOrigin ORIGIN (repeatable), -RegistryListen IP:PORT, -Extra "ARGS",
-NoticeFile FILE (the operator's notice, JSON from the hoi app's "A node on a server" page or the
setup app: what the node keeps and, optional but recommended, who runs it, how to reach them and
where the server is, shown to everyone who connects; copied next to the node and carried by it, so
people read it before it serves them; a public node without one says so).
-Yes asks nothing, -NoLaunch leaves the app closed, -DryRun changes nothing.
#>
[CmdletBinding()]
param(
  [ValidateSet('desktop', 'companion', 'relay', 'home', 'private', 'public', 'registry', 'uninstall')]
  [string]$Role,
  [string[]]$Node = @(),
  [string]$NoticeFile,
  [string[]]$Index = @(),
  [string]$Name,
  [double]$DailyCapGb = 0,
  [double]$HoldPiecesGb = -1,
  [string]$RtcAdvertise,
  [switch]$NoPortMapping,
  [string]$RegistryUrl,
  [string[]]$RegistryOrigin = @(),
  [string]$RegistryListen = '127.0.0.1:7845',
  [string]$Extra,
  [string]$Manifest = 'https://hoi.sh/releases/manifest.json',
  [string]$Release,
  [string]$Repo = 'hoi-project/hoi',
  [string]$From,
  [string]$Binary,
  # Filled from publication.json when preparing the published installer / Node Setup.
  [string]$PubKey = '',
  [switch]$AllowUnsignedLocal,
  [switch]$Yes,
  [switch]$NoLaunch,
  [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$TaskName = 'hoi-node'
$NodeHome = Join-Path $env:LOCALAPPDATA 'hoi-node'
$Target = 'x86_64-pc-windows-msvc'

function Fail([string]$Message) { throw "install.ps1: $Message" }
function Note([string]$Message) { Write-Warning $Message }
# Changes this computer: printed instead of run with -DryRun.
function Invoke-Change([string]$Description, [scriptblock]$Action) {
  if ($DryRun) { Write-Host "+ $Description" } else { & $Action }
}
function Interactive { -not $Yes -and [Environment]::UserInteractive -and -not [Console]::IsInputRedirected }

if (-not $Role) {
  if (-not (Interactive)) { Fail 'say what to install: -Role desktop, companion, home, private, public or registry' }
  Write-Host @'
What should this computer run?
  1  desktop    the hoi app: opens as the companion in the tray, the full app a click away
  2  companion  the same app, started in the tray whenever you log in
  3  home       a home node for your phone and browser, a public relay too, started when you log in
  4  private    a home node for your own devices only, started when you log in
  5  public     a public node (no key), started when you log in
  6  registry   a public node with a registry, started when you log in
'@
  $choice = Read-Host 'Choose 1-6'
  $picked = @{ '1' = 'desktop'; '2' = 'companion'; '3' = 'home'; '4' = 'private'; '5' = 'public'; '6' = 'registry' }[$choice]
  if (-not $picked) { Fail 'nothing chosen' }
  $Role = $picked
}
# Every home node is a public relay now; "relay" is the name that role had before.
if ($Role -eq 'relay') { $Role = 'home' }
$Kind = switch ($Role) { { $_ -in @('desktop', 'companion') } { 'app' } 'uninstall' { 'uninstall' } default { 'node' } }

if ($Role -eq 'registry' -and -not $RegistryUrl -and (Interactive)) {
  $RegistryUrl = Read-Host "The registry's public https:// address (your TLS proxy in front of it)"
}

# Everything below ends up on a command line: plain characters only.
$HttpsUrl = '^https://[A-Za-z0-9.-]+(:[0-9]+)?(/[A-Za-z0-9._~/-]*)?$'
if (-not $Name) { $Name = $env:COMPUTERNAME }
$Name = ($Name -replace "[^A-Za-z0-9 ._'-]", '')
if ($Name.Length -gt 40) { $Name = $Name.Substring(0, 40) }
if (-not $Name) { $Name = 'home node' }
foreach ($url in $Index) { if ($url -notmatch $HttpsUrl) { Fail "-Index needs an https:// registry URL without credentials, query or fragment: $url" } }
$NodeLink = '^(hoi://node\?|https://[A-Za-z0-9.-]+(:[0-9]+)?(/[A-Za-z0-9._~/-]*)?#/node\?)[A-Za-z0-9._~:@,=&%-]+$'
foreach ($link in $Node) { if ($link -notmatch $NodeLink) { Fail "-Node needs a node link (hoi://node?... or https://...#/node?...): $link" } }
if ($RegistryUrl -and $RegistryUrl -notmatch $HttpsUrl) { Fail "-RegistryUrl needs the registry's https:// address" }
if ($Role -eq 'registry' -and -not $RegistryUrl) { Fail 'a registry needs -RegistryUrl, its public https:// address' }
foreach ($o in $RegistryOrigin) {
  if ($o -notmatch '^(https://[A-Za-z0-9.-]+(:[0-9]+)?|tauri://localhost|http://tauri\.localhost)$') { Fail "-RegistryOrigin takes an exact https:// origin: $o" }
}
if ($RegistryOrigin.Count -eq 0) { $RegistryOrigin = @('https://hoi.sh', 'tauri://localhost', 'http://tauri.localhost') }
if ($RegistryListen -notmatch '^([0-9.]+|\[[0-9a-fA-F:]+\]):[0-9]+$') { Fail '-RegistryListen takes IP:PORT' }
if ($RtcAdvertise -and $RtcAdvertise -notmatch '^([0-9.]+|\[[0-9a-fA-F:]+\]):[0-9]+$') { Fail '-RtcAdvertise takes the public IP:PORT' }
if ($DailyCapGb -lt 0) { Fail '-DailyCapGb needs a number of gigabytes' }
if ($HoldPiecesGb -ne -1 -and $HoldPiecesGb -ne 0 -and ($HoldPiecesGb -lt 0.1 -or $HoldPiecesGb -gt 10000)) { Fail '-HoldPiecesGb needs a number of gigabytes from 0.1 to 10000, or 0 for none' }
if ($HoldPiecesGb -gt 0 -and $Role -and $Role -notin @('public', 'registry')) { Fail '-HoldPiecesGb is for a public node or a registry' }
if ($Extra -and $Extra -notmatch '^[][A-Za-z0-9 ._:/=,-]+$') { Fail '-Extra takes plain flags and values' }
if ($NoticeFile) {
  if ($Kind -ne 'node') { Fail '-NoticeFile goes with a node: it is the operator notice the node carries' }
  if (-not (Test-Path $NoticeFile)) { Fail "no such notice file: $NoticeFile" }
  $noticeText = Get-Content -Raw $NoticeFile
  # Only these are required; who and where (operator, contact, country) may be left out.
  foreach ($key in @('processes', 'updated')) {
    if ($noticeText -notmatch "`"$key`"") { Fail "$NoticeFile is no notice: no `"$key`" in it (the hoi app's A node on a server page and the setup app write one)" }
  }
}
if ($Kind -eq 'node' -and $Role -in @('public', 'registry') -and -not $NoticeFile) {
  Note 'no operator notice (-NoticeFile): people who connect are not told who runs this node, where, or how to reach you (hoi.sh/download/#operator)'
}
if ($Release) {
  if ($Release -notmatch '^v[0-9A-Za-z.-]+$') { Fail '-Release takes a tag such as v0.2.0' }
  if ($Repo -notmatch '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$') { Fail '-Repo takes OWNER/NAME' }
  if (-not (Get-Command gh -ErrorAction SilentlyContinue)) { Fail '-Release needs the GitHub CLI (gh), signed in' }
}
if ($From -and -not (Test-Path (Join-Path $From 'manifest.json'))) { Fail "$From has no manifest.json (a release folder from scripts/release-local.mjs has one)" }
if ($Binary -and $Kind -ne 'node') { Fail '-Binary is a hoi-node; for the app, use -From with a release folder' }

$Work = Join-Path ([IO.Path]::GetTempPath()) ("hoi-install-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $Work | Out-Null

# ── Release files ──────────────────────────────────────────────────────────────────────────

$script:List = $null
$script:InstalledVersion = ''
$ReleaseStamp = Join-Path $(if ($Kind -eq 'app') { Join-Path $env:LOCALAPPDATA 'hoi' } else { $NodeHome }) 'release-version'
function Compare-ReleaseVersion([string]$Left, [string]$Right) {
  $a = [regex]::Split(($Left -split '\+')[0], '-', 2)
  $b = [regex]::Split(($Right -split '\+')[0], '-', 2)
  $core = ([version]$a[0]).CompareTo([version]$b[0])
  if ($core -ne 0) { return $core }
  if ($a.Count -eq 1 -and $b.Count -eq 1) { return 0 }
  if ($a.Count -eq 1) { return 1 }
  if ($b.Count -eq 1) { return -1 }
  $x = $a[1] -split '\.'; $y = $b[1] -split '\.'
  for ($i = 0; $i -lt [Math]::Max($x.Count, $y.Count); $i++) {
    if ($i -ge $x.Count) { return -1 }; if ($i -ge $y.Count) { return 1 }
    if ($x[$i] -ceq $y[$i]) { continue }
    $xn = $x[$i] -match '^\d+$'; $yn = $y[$i] -match '^\d+$'
    if ($xn -and $yn) { return ([System.Numerics.BigInteger]::Parse($x[$i])).CompareTo([System.Numerics.BigInteger]::Parse($y[$i])) }
    if ($xn -ne $yn) { if ($xn) { return -1 } else { return 1 } }
    return [string]::CompareOrdinal($x[$i], $y[$i])
  }
  return 0
}
function Assert-NoDowngrade([string]$Version) {
  $current = ''
  if (Test-Path $ReleaseStamp) { $current = (Get-Content -Raw $ReleaseStamp).Trim() }
  elseif ($Kind -ne 'app' -and (Test-Path (Join-Path $NodeHome 'hoi-node.exe'))) { $current = ((& (Join-Path $NodeHome 'hoi-node.exe') --version) -split ' ')[1] }
  if ($current -and (Compare-ReleaseVersion $Version $current) -lt 0) { Fail "refusing to downgrade hoi from $current to $Version; nothing was installed" }
}
function Get-ReleaseList {
  if ($script:List) { return $script:List }
  $path = Join-Path $Work 'manifest.json'
  if ($From) {
    Copy-Item (Join-Path $From 'manifest.json') $path
    $sig = Join-Path $From 'manifest.json.minisig'
    if (Test-Path $sig) { Copy-Item $sig $Work }
  } elseif ($Release) {
    & gh release download $Release --repo $Repo --pattern 'manifest.json*' --dir $Work | Out-Host
    if ($LASTEXITCODE -ne 0) { Fail "release $Release in $Repo has no manifest.json (gh signed in?)" }
  } else {
    if ($Manifest -notmatch '^https://') { Fail "refusing to download over anything but https: $Manifest" }
    Invoke-WebRequest -UseBasicParsing -Uri $Manifest -OutFile $path
    try { Invoke-WebRequest -UseBasicParsing -Uri "$Manifest.minisig" -OutFile "$path.minisig" } catch { }
  }
  if ((Get-Item $path).Length -gt 1048576) { Fail 'release list exceeds 1 MiB' }
  if ($AllowUnsignedLocal -and -not $From) { Fail '-AllowUnsignedLocal requires a local -From folder; it never permits unsigned downloads' }
  if ($AllowUnsignedLocal -and $From -and -not $PubKey) {
    Note 'local development folder explicitly trusted: SHA-256 only; publisher not verified'
  } elseif (-not $PubKey) {
    Fail 'release signing key is not configured; use a published installer with a trusted key, -PubKey, or your own local binary'
  } elseif (-not (Test-Path "$path.minisig")) {
    Fail 'the release list has no signature (manifest.json.minisig)'
  } elseif (Get-Command minisign -ErrorAction SilentlyContinue) {
    & minisign -Vqm $path -x "$path.minisig" -P $PubKey | Out-Host
    if ($LASTEXITCODE -ne 0) { Fail "the release list's signature does not verify" }
    Write-Host 'release list: signature good'
  } else {
    Fail 'minisign is required to verify this release; install it and retry. Nothing was installed'
  }
  $list = Get-Content -Raw $path | ConvertFrom-Json
  if ($list.schema -ne 1 -or $list.releases.Count -ne 1 -or $list.releases[0].version -ne $list.latest -or $list.latest -notmatch '^\d+\.\d+\.\d+(-[A-Za-z0-9.-]+)?(\+[A-Za-z0-9.-]+)?$') { Fail 'the signed release list has an invalid schema or version' }
  if ($Release -and $Release -ne "v$($list.latest)") { Fail 'release tag does not match the signed list' }
  Assert-NoDowngrade $list.latest
  $script:InstalledVersion = $list.latest
  Write-Host "hoi $($list.latest)"
  $script:List = $list
  return $list
}

function Get-ReleaseFiles { (Get-ReleaseList).releases[0].files }

# Fetches one file of the release into the work folder and checks it; returns its path.
function Save-ReleaseFile($File) {
  $name = [string]$File.name
  if ($name -notmatch '^[A-Za-z0-9._+-]+$') { Fail "an odd file name in the release list: $name" }
  if ([string]$File.sha256 -notmatch '^[0-9a-f]{64}$') { Fail "no SHA-256 for $name in the release list" }
  $dest = Join-Path $Work $name
  Write-Host "fetching $name"
  if ($From) {
    Copy-Item (Join-Path $From $name) $dest
  } elseif ($Release) {
    & gh release download $Release --repo $Repo --pattern $name --dir $Work --clobber | Out-Host
    if ($LASTEXITCODE -ne 0) { Fail "could not fetch $name" }
  } else {
    $url = [string]$File.url
    if ($url -notmatch '^https://') { Fail "refusing to download over anything but https: $url" }
    try {
      Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $dest
    } catch {
      # A private repository's release files need a signed-in GitHub CLI.
      if ($url -match '^https://github\.com/([^/]+/[^/]+)/releases/download/([^/]+)/' -and (Get-Command gh -ErrorAction SilentlyContinue)) {
        & gh release download $Matches[2] --repo $Matches[1] --pattern $name --dir $Work --clobber | Out-Host
        if ($LASTEXITCODE -ne 0) { Fail "could not fetch $name" }
      } else {
        Fail "could not fetch $url"
      }
    }
  }
  if ((Get-FileHash -Algorithm SHA256 $dest).Hash.ToLowerInvariant() -ne $File.sha256) {
    Fail "$name does not match the release list's SHA-256: not installing it"
  }
  return $dest
}

# ── The app ────────────────────────────────────────────────────────────────────────────────

function Find-App {
  foreach ($dir in @((Join-Path $env:LOCALAPPDATA 'hoi'), (Join-Path $env:LOCALAPPDATA 'Programs\hoi'), (Join-Path $env:ProgramFiles 'hoi'))) {
    if (Test-Path $dir) {
      $exe = Get-ChildItem -Path $dir -Filter '*.exe' | Where-Object { $_.Name -notmatch '^uninstall' } | Select-Object -First 1
      if ($exe) { return $exe.FullName }
    }
  }
  return $null
}

function Install-App {
  # The node setup app's files (product "setup") are not the app.
  $files = Get-ReleaseFiles | Where-Object { $_.product -ne 'setup' }
  $setup = $files | Where-Object { $_.platform -eq 'windows' -and $_.kind -eq 'exe' } | Select-Object -First 1
  $msi = $files | Where-Object { $_.platform -eq 'windows' -and $_.kind -eq 'msi' } | Select-Object -First 1
  if (-not $setup -and -not $msi) { Fail 'this release has no app for Windows' }
  $running = Get-Process -Name 'hoi-desktop' -ErrorAction SilentlyContinue
  if ($running) {
    Write-Host 'closing the running hoi to replace it'
    Invoke-Change 'stop hoi-desktop' { $running | Stop-Process -Force; Start-Sleep -Seconds 2 }
  }
  if ($setup) {
    $path = Save-ReleaseFile $setup
    # The setup installs for this account only (no administrator needed); /S asks nothing.
    Invoke-Change "$path /S" { Start-Process -FilePath $path -ArgumentList '/S' -Wait }
  } else {
    $path = Save-ReleaseFile $msi
    Invoke-Change "msiexec /i $path /qn" { Start-Process -FilePath 'msiexec.exe' -ArgumentList @('/i', "`"$path`"", '/qn') -Wait }
  }
  $exe = Find-App
  if ($DryRun) { $exe = Join-Path $env:LOCALAPPDATA 'hoi\hoi-desktop.exe' }
  if (-not $exe) { Fail 'the app was installed, but its program was not found; start hoi from the Start menu' }
  Write-Host "installed $exe"
  if ($Role -eq 'companion') {
    Invoke-Change "$exe --hidden --start-at-login" { Start-Process -FilePath $exe -ArgumentList @('--hidden', '--start-at-login') }
    Write-Host "hoi starts in the tray now and whenever you log in (the companion's switch turns it off)"
  } elseif (-not $NoLaunch) {
    Invoke-Change "$exe" { Start-Process -FilePath $exe }
  }
}

# ── Nodes ──────────────────────────────────────────────────────────────────────────────────

# A key holder's data and a public node's are kept apart, so switching roles never hands a
# person's key to a public node.
function Get-NodeData { if ($Role -in @('public', 'registry')) { Join-Path $NodeHome 'public' } else { Join-Path $NodeHome 'data' } }

function Get-NodeArgs([string]$Data) {
  $a = @('--data', $Data, '--name', $Name, '--peer-bind', '0.0.0.0:7843', '--rtc-port', '7844')
  switch ($Role) {
    # A home node is a public relay unless told otherwise; nothing to add.
    'home' { if ($NoPortMapping) { $a += '--no-port-mapping' } }
    'private' { $a += '--no-public-relay' }
    'public' { $a += '--public'; if (-not $NoPortMapping) { $a += '--port-mapping' } }
    'registry' {
      $a += @('--public', '--registry', $RegistryListen, '--registry-url', $RegistryUrl)
      foreach ($o in $RegistryOrigin) { $a += @('--registry-origin', $o) }
      # It announces itself to its own registry and carries what that lists.
      $a += @('--index', $RegistryUrl)
      if (-not $NoPortMapping) { $a += '--port-mapping' }
    }
  }
  foreach ($url in $Index) { if ($url -ne $RegistryUrl) { $a += @('--index', $url) } }
  foreach ($link in $Node) { $a += @('--node', $link) }
  if ($script:NoticeInstalled) { $a += @('--notice', $script:NoticeInstalled) }
  if ($DailyCapGb -gt 0) { $a += @('--daily-cap-gb', [string]$DailyCapGb) }
  # not given: hoi-node holds 2 GB on a node that serves strangers; 0 turns that off
  if ($HoldPiecesGb -ge 0) { $a += @('--hold-pieces', [string]$HoldPiecesGb) }
  if ($RtcAdvertise) { $a += @('--rtc-advertise', $RtcAdvertise) }
  if ($Extra) { $a += ($Extra -split ' ' | Where-Object { $_ }) }
  return $a
}

function Get-NodeBinary {
  $exe = Join-Path $Work 'hoi-node.exe'
  if ($Binary) {
    if (-not (Test-Path $Binary)) { Fail "no such file: $Binary" }
    if ($Binary -match '\.(tar\.gz|tgz)$') { & tar -xzf $Binary -C $Work hoi-node.exe | Out-Host } else { Copy-Item $Binary $exe }
  } else {
    $file = Get-ReleaseFiles | Where-Object { $_.platform -eq 'node' -and $_.name -like "*_$Target.tar.gz" } | Select-Object -First 1
    if (-not $file) { Fail "this release has no hoi-node for Windows ($Target)" }
    $archive = Save-ReleaseFile $file
    & tar -xzf $archive -C $Work hoi-node.exe | Out-Host
  }
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path $exe)) { Fail 'no hoi-node.exe in that archive' }
  $actualVersion = ((& $exe --version) -split ' ')[1]
  if ($LASTEXITCODE -ne 0) { Fail 'that hoi-node does not run on this computer' }
  if ($actualVersion -notmatch '^\d+\.\d+\.\d+(-[A-Za-z0-9.-]+)?(\+[A-Za-z0-9.-]+)?$') { Fail 'hoi-node has no valid version' }
  if ($script:InstalledVersion -and $actualVersion -ne $script:InstalledVersion) { Fail 'hoi-node version does not match the signed release list' }
  Assert-NoDowngrade $actualVersion
  $script:InstalledVersion = $actualVersion
  return $exe
}

function Stop-Node {
  if (Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue) {
    Invoke-Change "stop and remove the scheduled task $TaskName" {
      Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
      Unregister-ScheduledTask -TaskName $TaskName -Confirm:$false
    }
  }
  $running = Get-Process -Name 'hoi-node' -ErrorAction SilentlyContinue
  if ($running) { Invoke-Change 'stop hoi-node' { $running | Stop-Process -Force; Start-Sleep -Seconds 1 } }
}

function Install-Node {
  if ($Role -eq 'uninstall') {
    Stop-Node
    Invoke-Change "remove $NodeHome\hoi-node.exe" { Remove-Item -Force -ErrorAction SilentlyContinue (Join-Path $NodeHome 'hoi-node.exe'), (Join-Path $NodeHome 'notice.json') }
    Write-Host "removed the hoi-node task; its data stays in $NodeHome"
    return
  }
  $built = Get-NodeBinary
  if ($NoticeFile) {
    & $built admin notice check $NoticeFile | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail "the notice in $NoticeFile cannot be used (hoi-node admin notice check $NoticeFile says why)" }
  }
  Stop-Node
  $exe = Join-Path $NodeHome 'hoi-node.exe'
  $data = Get-NodeData
  $log = Join-Path $NodeHome 'hoi-node.log'
  $notice = Join-Path $NodeHome 'notice.json'
  Invoke-Change "install $exe" {
    New-Item -ItemType Directory -Force -Path $NodeHome, $data | Out-Null
    Copy-Item -Force $built $exe
  }
  if ($NoticeFile) {
    Invoke-Change "install the operator notice $notice" { Copy-Item -Force $NoticeFile $notice }
    $script:NoticeInstalled = $notice
  } elseif (-not $DryRun -and (Test-Path $notice)) {
    $script:NoticeInstalled = $notice
    Note "keeping the operator notice from the last install ($notice)"
  }
  # PowerShell runs the node hidden at logon and appends what it says to the log. Single quotes
  # are doubled: nothing in a value can end its string early.
  $quoted = (Get-NodeArgs $data | ForEach-Object { "'" + ($_ -replace "'", "''") + "'" }) -join ' '
  $command = "& '$($exe -replace "'", "''")' $quoted *>> '$($log -replace "'", "''")'"
  $argument = "-NoProfile -NonInteractive -WindowStyle Hidden -Command `"$command`""
  Invoke-Change "register the scheduled task ${TaskName}: powershell.exe $argument" {
    $action = New-ScheduledTaskAction -Execute 'powershell.exe' -Argument $argument
    $trigger = New-ScheduledTaskTrigger -AtLogOn -User "$env:USERDOMAIN\$env:USERNAME"
    $settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -StartWhenAvailable `
      -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
    Register-ScheduledTask -TaskName $TaskName -Action $action -Trigger $trigger -Settings $settings `
      -Description "hoi-node ($Role), written by install.ps1; run it again to change this" -Force | Out-Null
    Start-ScheduledTask -TaskName $TaskName
  }
  Write-Host ''
  Write-Host "hoi-node ($Role) runs now and starts whenever you log in."
  Write-Host "  status:          Get-Content -Wait '$log'"
  if ($Role -in @('home', 'private')) { Write-Host "  pair your phone: Get-Content '$data\pairing-link'   (open the https:// link on it)" }
  if ($Role -ne 'private') { Write-Host "  share the node:  Get-Content '$data\node-link'   (anyone who opens it finds what it carries)" }
  if ($script:NoticeInstalled) { Write-Host "  your notice:     $script:NoticeInstalled   (edit it; the node reads it again within half a minute)" }
  Write-Host "  stop:            Stop-Process -Name hoi-node   (it starts again when you log in; Start-ScheduledTask $TaskName starts it now)"
  Write-Host 'If Windows asks whether hoi-node may use the network, allow it (private and public networks for a public node).'
  if ($Role -ne 'private') { Write-Host 'No registry is needed: apps and other nodes reach this one through its node link, the invites of the communities it carries, and the public nodes that pass it on.' }
  if ($Role -eq 'registry') {
    Write-Host ''
    Write-Host "The registry listens on $RegistryListen. Browsers reach a registry only over https, so put a TLS proxy"
    Write-Host "in front of it at $RegistryUrl (for example: caddy reverse-proxy --from $($RegistryUrl -replace '^https://', '') --to $RegistryListen)"
    Write-Host 'The published app uses no registry; only an app built for this one reaches it (registry/README.md).'
  }
}

try {
  if ($Kind -eq 'app') { Install-App } else { Install-Node }
  if ($script:InstalledVersion -and $Role -ne 'uninstall') {
    Invoke-Change "record installed release $script:InstalledVersion" {
      New-Item -ItemType Directory -Force -Path (Split-Path $ReleaseStamp) | Out-Null
      Set-Content -Path $ReleaseStamp -Value $script:InstalledVersion -Encoding ascii
    }
  }
} finally {
  Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Work
}
