# Install the latest CodeDna release binary on Windows.
#
#   irm https://raw.githubusercontent.com/crafteraadarsh/codedna/main/install.ps1 | iex
#
# Environment:
#   CODEDNA_VERSION      release tag to install (default: latest), e.g. v0.2.0
#   CODEDNA_INSTALL_DIR  where to put the binary (default: %LOCALAPPDATA%\codedna\bin)
#
# The archive's SHA-256 checksum is verified before anything is installed.

$ErrorActionPreference = 'Stop'

$repo = 'crafteraadarsh/codedna'
$version = if ($env:CODEDNA_VERSION) { $env:CODEDNA_VERSION } else { 'latest' }
$installDir = if ($env:CODEDNA_INSTALL_DIR) { $env:CODEDNA_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'codedna\bin' }

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'codedna install: only 64-bit Windows is supported.'
}

$target = 'x86_64-pc-windows-msvc'
$archive = "codedna-$target.zip"
$base = if ($version -eq 'latest') {
    "https://github.com/$repo/releases/latest/download"
} else {
    "https://github.com/$repo/releases/download/$version"
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Write-Host "Downloading $archive ($version)..."
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -UseBasicParsing "$base/$archive" -OutFile (Join-Path $tmp $archive)
    Invoke-WebRequest -UseBasicParsing "$base/$archive.sha256" -OutFile (Join-Path $tmp "$archive.sha256")

    $expected = ((Get-Content (Join-Path $tmp "$archive.sha256") -Raw).Trim() -split '\s+')[0].ToLower()
    $actual = (Get-FileHash (Join-Path $tmp $archive) -Algorithm SHA256).Hash.ToLower()
    if (-not $expected -or $expected -ne $actual) {
        throw "codedna install: checksum mismatch for $archive; refusing to install."
    }

    Expand-Archive (Join-Path $tmp $archive) -DestinationPath $tmp
    New-Item -ItemType Directory -Force $installDir | Out-Null
    Copy-Item (Join-Path $tmp "codedna-$target\codedna.exe") (Join-Path $installDir 'codedna.exe') -Force

    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $installDir) {
        [Environment]::SetEnvironmentVariable('Path', "$userPath;$installDir", 'User')
        Write-Host "Added $installDir to your PATH. Open a new terminal to use it."
    }
    Write-Host "Installed $(& (Join-Path $installDir 'codedna.exe') --version) to $installDir"
} finally {
    Remove-Item -Recurse -Force $tmp
}
