# Copies the Tauri front end into site/demo/ui and points it at mock.js, so the
# website runs the real flyout in the browser.
#
#   pwsh scripts/build-demo.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$from = Join-Path $root 'apps/tauri/ui'
$to = Join-Path $root 'site/demo/ui'

New-Item -ItemType Directory -Force $to | Out-Null
Copy-Item (Join-Path $from '*') $to -Recurse -Force

$index = Join-Path $to 'index.html'
$html = Get-Content $index -Raw
# The settings panel shows the app's version, so the demo takes it from Cargo.toml.
$version = (Select-String -Path (Join-Path $root 'Cargo.toml') -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Set-Content (Join-Path $to 'version.js') "window.__abVersion = `"$version`";" -NoNewline

$shim = "<script src=`"version.js`"></script>`n  <script src=`"../trayicon.js`"></script>`n  <script src=`"../mock.js`"></script>`n  "
$html = $html -replace '(<script type="module" src="app.js"></script>)', "$shim`$1"
if ($html -notmatch 'mock\.js') { throw 'index.html no longer loads app.js the expected way' }
Set-Content $index $html -NoNewline
Write-Host "Demo UI $version copied to $to"
