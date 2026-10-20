$ErrorActionPreference = 'Stop'
$repo = if ($env:FORWIROH_REPOSITORY) { $env:FORWIROH_REPOSITORY } else { 'trufae/forwiroh' }
$version = $env:FORWIROH_VERSION
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
if (-not $version) {
    $version = (Invoke-RestMethod "https://api.github.com/repos/$repo/releases/latest").tag_name
}
if ($version -notmatch '^v[0-9]+\.[0-9]+\.[0-9]+$') { throw "Invalid release tag: $version" }
$architecture = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
$target = switch ($architecture) {
    'X64' { 'x86_64-pc-windows-msvc' }
    'Arm64' { 'aarch64-pc-windows-msvc' }
    default { throw "Unsupported Windows architecture: $architecture" }
}
$asset = "forwiroh-$target.zip"
$base = "https://github.com/$repo/releases/download/$version"
$destination = if ($env:FORWIROH_INSTALL_DIR) { $env:FORWIROH_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'forwiroh\bin' }
$temporary = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory $temporary | Out-Null
try {
    $checksums = (Invoke-WebRequest -UseBasicParsing "$base/CLI-SHA256SUMS").Content
    $checksumMatches = [regex]::Matches($checksums, "(?m)^([a-fA-F0-9]{64})  $([regex]::Escape($asset))`r?$")
    if ($checksumMatches.Count -ne 1) { throw "Missing/invalid checksum for $asset" }
    $archive = Join-Path $temporary $asset
    Invoke-WebRequest -UseBasicParsing "$base/$asset" -OutFile $archive
    if ((Get-FileHash $archive -Algorithm SHA256).Hash -ne $checksumMatches[0].Groups[1].Value) {
        throw 'SHA-256 mismatch; installation aborted'
    }
    Expand-Archive $archive -DestinationPath $temporary
    $binary = Join-Path $temporary 'forwiroh.exe'
    & $binary --version
    if ($LASTEXITCODE -ne 0) { throw 'Downloaded CLI failed to run' }
    New-Item -ItemType Directory -Force $destination | Out-Null
    Copy-Item $binary (Join-Path $destination 'forwiroh.exe') -Force
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (($userPath -split ';') -notcontains $destination) {
        [Environment]::SetEnvironmentVariable('Path', "$destination;$userPath", 'User')
    }
    if (($env:Path -split ';') -notcontains $destination) { $env:Path = "$destination;$env:Path" }
    Write-Host "Installed $version to $destination\forwiroh.exe"
} finally {
    Remove-Item $temporary -Recurse -Force
}
