param([Parameter(Mandatory = $true)][string]$Archive)
$ErrorActionPreference = 'Stop'
$fixtureRoot = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString())
New-Item -ItemType Directory $fixtureRoot | Out-Null
$env:FORWIROH_TEST_FIXTURE = $fixtureRoot
$oldDestination = $env:FORWIROH_INSTALL_DIR
$oldVersion = $env:FORWIROH_VERSION
$oldProcessPath = $env:Path
$oldUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
try {
    $fixtureArchive = Join-Path $fixtureRoot (Split-Path $Archive -Leaf)
    Copy-Item $Archive $fixtureArchive
    $fixtureChecksum = (Get-FileHash $fixtureArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    [IO.File]::WriteAllText((Join-Path $fixtureRoot 'CLI-SHA256SUMS'), "$fixtureChecksum  $(Split-Path $Archive -Leaf)`n")
    function Invoke-WebRequest {
        param([string]$Uri, [switch]$UseBasicParsing, [string]$OutFile)
        if ($Uri -notlike 'https://github.com/trufae/forwiroh/releases/download/v0.1.1/*') {
            throw "Unexpected download: $Uri"
        }
        $source = Join-Path $env:FORWIROH_TEST_FIXTURE ($Uri.Split('/')[-1])
        if ($OutFile) { Copy-Item $source $OutFile } else { [pscustomobject]@{Content = [IO.File]::ReadAllText($source)} }
    }
    $env:FORWIROH_VERSION = 'v0.1.1'
    $env:FORWIROH_INSTALL_DIR = Join-Path $fixtureRoot 'install with spaces'
    & "$PSScriptRoot/../install.ps1"
    $installedBinary = Join-Path $env:FORWIROH_INSTALL_DIR 'forwiroh.exe'
    if (-not (Test-Path $installedBinary)) { throw 'CLI was not installed' }
    $installedChecksum = (Get-FileHash $installedBinary -Algorithm SHA256).Hash
    # A corrupt update must leave the working executable untouched.
    [IO.File]::AppendAllText($fixtureArchive, 'corrupted')
    $rejected = $false
    try { & "$PSScriptRoot/../install.ps1" } catch {
        if ($_.Exception.Message -notlike '*SHA-256 mismatch*') { throw }
        $rejected = $true
    }
    if (-not $rejected) { throw 'Corrupt update was accepted' }
    if ((Get-FileHash $installedBinary -Algorithm SHA256).Hash -ne $installedChecksum) {
        throw 'Corrupt update replaced the installed CLI'
    }
    Write-Host 'PowerShell installer and corrupt-update rejection: PASS'
} finally {
    [Environment]::SetEnvironmentVariable('Path', $oldUserPath, 'User')
    $env:Path = $oldProcessPath
    $env:FORWIROH_INSTALL_DIR = $oldDestination
    $env:FORWIROH_VERSION = $oldVersion
    Remove-Item Env:FORWIROH_TEST_FIXTURE
    Remove-Item $fixtureRoot -Recurse -Force
}
