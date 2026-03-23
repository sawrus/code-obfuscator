param(
    [switch]$FromSource,
    [string]$Binary,
    [string]$Archive,
    [string]$Version,
    [switch]$NoModifyPath
)

$ErrorActionPreference = 'Stop'
$App = 'code-obfuscator'
$InstallHome = if ($env:CODE_OBFUSCATOR_HOME) { $env:CODE_OBFUSCATOR_HOME } else { Join-Path $HOME '.code-obfuscator' }
$InstallDir = if ($env:CODE_OBFUSCATOR_INSTALL_DIR) { $env:CODE_OBFUSCATOR_INSTALL_DIR } elseif ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA 'Programs\code-obfuscator\bin' } else { Join-Path $HOME 'AppData\Local\Programs\code-obfuscator\bin' }
$ManagedBinDir = Join-Path $InstallHome 'bin'
$ReleasesDir = Join-Path $InstallHome 'releases'
$VersionFile = Join-Path $InstallHome 'current-version'
$InstallRepo = $env:CODE_OBFUSCATOR_INSTALL_REPO
$InstallBaseUrl = $env:CODE_OBFUSCATOR_INSTALL_BASE_URL

function Write-Info([string]$Message) { Write-Host $Message }
function Ensure-Dir([string]$Path) { New-Item -ItemType Directory -Path $Path -Force | Out-Null }
function Require-Cmd([string]$Name) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "required command not found: $Name"
    }
}
function Install-Payload([string]$PayloadBinary, [string]$ResolvedVersion) {
    $releaseDir = Join-Path $ReleasesDir $ResolvedVersion
    $releaseBinDir = Join-Path $releaseDir 'bin'
    $managedBinary = Join-Path $ManagedBinDir "$App.exe"
    $installedBinary = Join-Path $releaseBinDir "$App.exe"

    Ensure-Dir $releaseBinDir
    Copy-Item $PayloadBinary $installedBinary -Force
    Copy-Item $installedBinary $managedBinary -Force
    Copy-Item $managedBinary (Join-Path $InstallDir "$App.exe") -Force
    Set-Content -Path $VersionFile -Value $ResolvedVersion

    Write-Info "Installed $App $ResolvedVersion into $InstallHome"
    Write-Info "Executable available at $(Join-Path $InstallDir "$App.exe")"
}
function Resolve-LatestVersion() {
    if (-not $InstallRepo) { return $null }
    $url = "https://api.github.com/repos/$InstallRepo/releases/latest"
    $json = Invoke-RestMethod -Uri $url
    return ($json.tag_name -replace '^v', '')
}
function Resolve-ArchiveName() {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString().ToLowerInvariant()
    if ($arch -eq 'x64') { $arch = 'x64' }
    elseif ($arch -eq 'arm64') { $arch = 'arm64' }
    else { throw "unsupported architecture: $arch" }

    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::Windows)) { return "$App-windows-$arch.zip" }
    if ([System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform([System.Runtime.InteropServices.OSPlatform]::OSX)) { return "$App-darwin-$arch.tar.gz" }
    return "$App-linux-$arch.tar.gz"
}
function Download-And-Install([string]$ResolvedVersion) {
    $archiveName = Resolve-ArchiveName
    if ($InstallBaseUrl) {
        $url = "$($InstallBaseUrl.TrimEnd('/'))/v$ResolvedVersion/$archiveName"
    } elseif ($InstallRepo) {
        $url = "https://github.com/$InstallRepo/releases/download/v$ResolvedVersion/$archiveName"
    } else {
        throw 'release download source is not configured; set CODE_OBFUSCATOR_INSTALL_REPO or CODE_OBFUSCATOR_INSTALL_BASE_URL'
    }

    $tempDir = New-Item -ItemType Directory -Path (Join-Path ([System.IO.Path]::GetTempPath()) "$App.install.$([Guid]::NewGuid())") -Force
    try {
        $archiveFile = Join-Path $tempDir.FullName $archiveName
        Write-Info "Downloading $url"
        Invoke-WebRequest -Uri $url -OutFile $archiveFile
        $script:Archive = $archiveFile
        Install-FromArchive $ResolvedVersion
    } finally {
        Remove-Item $tempDir.FullName -Recurse -Force -ErrorAction SilentlyContinue
    }
}
function Install-FromArchive([string]$ResolvedVersion) {
    if (-not (Test-Path $Archive)) { throw "archive not found: $Archive" }
    $tempDir = New-Item -ItemType Directory -Path (Join-Path ([System.IO.Path]::GetTempPath()) "$App.extract.$([Guid]::NewGuid())") -Force
    try {
        if ($Archive.EndsWith('.zip')) {
            Expand-Archive -Path $Archive -DestinationPath $tempDir.FullName -Force
        } else {
            Require-Cmd tar
            & tar -xzf $Archive -C $tempDir.FullName
        }
        $payload = Get-ChildItem -Path $tempDir.FullName -Recurse -Filter "$App*.exe" | Select-Object -First 1
        if (-not $payload) { throw "could not locate $App executable in archive" }
        if ($ResolvedVersion) {
            Install-Payload $payload.FullName $ResolvedVersion
        } else {
            Install-Payload $payload.FullName 'archive'
        }
    } finally {
        Remove-Item $tempDir.FullName -Recurse -Force -ErrorAction SilentlyContinue
    }
}
function Maybe-UpdatePath() {
    if ($NoModifyPath) { return }
    $currentUser = [Environment]::GetEnvironmentVariable('PATH', 'User')
    $entries = @()
    if ($currentUser) { $entries = $currentUser -split ';' }
    if ($entries -contains $InstallDir) { return }
    $newPath = if ([string]::IsNullOrWhiteSpace($currentUser)) { $InstallDir } else { "$InstallDir;$currentUser" }
    [Environment]::SetEnvironmentVariable('PATH', $newPath, 'User')
    Write-Info "Added $InstallDir to user PATH"
}

Ensure-Dir $InstallHome
Ensure-Dir $InstallDir
Ensure-Dir $ManagedBinDir
Ensure-Dir $ReleasesDir

$selectionCount = 0
if ($FromSource) { $selectionCount++ }
if ($Binary) { $selectionCount++ }
if ($Archive) { $selectionCount++ }
if ($selectionCount -gt 1) { throw 'choose only one of -FromSource, -Binary, or -Archive' }

if ($Binary) {
    Install-Payload $Binary ($(if ($Version) { $Version } else { 'local' }))
} elseif ($Archive) {
    Install-FromArchive ($(if ($Version) { $Version } else { 'archive' }))
} elseif ($FromSource) {
    Require-Cmd cargo
    $repoRoot = Resolve-Path (Join-Path $PSScriptRoot '..')
    & cargo build --release --bin $App --manifest-path (Join-Path $repoRoot 'Cargo.toml')
    Install-Payload (Join-Path $repoRoot "target\release\$App.exe") ($(if ($Version) { $Version } else { (Select-String -Path (Join-Path $repoRoot 'Cargo.toml') -Pattern '^version = "([^"]+)"').Matches[0].Groups[1].Value }))
} else {
    if (-not $Version) { $Version = Resolve-LatestVersion }
    if (-not $Version) { throw 'no installation source selected. Use -FromSource, -Binary, -Archive, or configure CODE_OBFUSCATOR_INSTALL_REPO' }
    Download-And-Install $Version
}

Maybe-UpdatePath
Write-Info "Try: $App --help"
