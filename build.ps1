#Requires -Version 7

<#
.SYNOPSIS
    Builds both front ends in release mode, copies the portable exes to dist/, and
    builds the installer with a pinned, hash-checked Inno Setup.
.PARAMETER Version
    Version stamped into the installer. Defaults to the workspace version in Cargo.toml,
    and must match it, so a tag can't ship a binary that reports another version.
.PARAMETER OutDir
    Where the exes and the installer go. Defaults to dist/.
#>
param(
    [string]$Version,
    [string]$OutDir
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$root  = $PSScriptRoot
$tools = Join-Path $root '.tools'
$dist  = if ($OutDir) { [IO.Path]::GetFullPath($OutDir) } else { Join-Path $root 'dist' }

$cargoVersion = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $Version) { $Version = $cargoVersion }
# Inno and the Windows version resource accept only numeric versions.
if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Version must look like 1.2.3, got '$Version'. Tag releases as v1.2.3."
}
if ($Version -ne $cargoVersion) {
    throw "Version $Version doesn't match Cargo.toml ($cargoVersion). Bump the workspace version first."
}

# Bump the version and the hash together.
$inno = @{
    Url    = 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_4/innosetup-6.7.4.exe'
    Sha256 = 'a549929319f9120716677c685535ca4671e4b937b5fdc87e2585f32d8eb30e8c'
    Dir    = 'inno'
}

New-Item -ItemType Directory -Force $tools, $dist | Out-Null

$file = Join-Path $tools (Split-Path $inno.Url -Leaf)
$innoDir = Join-Path $tools $inno.Dir
if (-not (Test-Path $file)) {
    Write-Host "Downloading $($inno.Url)"
    Invoke-WebRequest $inno.Url -OutFile $file
}
$hash = (Get-FileHash $file -Algorithm SHA256).Hash
if ($hash -ne $inno.Sha256) {
    Remove-Item $file
    throw "Hash mismatch for $file. Expected $($inno.Sha256), got $hash."
}
if (-not (Test-Path $innoDir)) {
    # Portable mode unpacks Inno Setup without registering it on the system.
    $innoArgs = '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/PORTABLE=1', "/DIR=`"$innoDir`""
    $p = Start-Process $file -ArgumentList $innoArgs -Wait -PassThru
    if ($p.ExitCode -ne 0) { throw "Inno Setup unpack exited with code $($p.ExitCode)." }
}

cargo build --release --locked -p audio_bartender_gpui -p audio_bartender_tauri
if ($LASTEXITCODE -ne 0) { throw "cargo build exited with code $LASTEXITCODE." }

$exes = foreach ($app in 'gpui', 'tauri') {
    $exe = Join-Path $dist "audio_bartender_$app.exe"
    Copy-Item (Join-Path $root "target/release/audio_bartender_$app.exe") $exe -Force
    $exe
}

$setup = Join-Path $dist 'audio_bartender_setup.exe'
Remove-Item $setup -ErrorAction SilentlyContinue
& (Join-Path $innoDir 'ISCC.exe') /Q "/DAppVersion=$Version" "/DDistDir=$dist" (Join-Path $root 'installer/audio_bartender.iss')
if ($LASTEXITCODE -ne 0) { throw "ISCC exited with code $LASTEXITCODE." }

foreach ($file in @($exes) + $setup) {
    $hash = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
    Write-Host "Built $file ($Version)"
    Write-Host "SHA256 $hash"
}
