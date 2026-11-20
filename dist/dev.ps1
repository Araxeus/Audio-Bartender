# Starts both front ends from this folder side by side. --dev turns off the
# single-instance lock and the hand-off to the elevated task.
#
#   pwsh dist/dev.ps1                       # tray only
#   pwsh dist/dev.ps1 --pinned              # with the flyouts open and pinned

foreach ($app in 'gpui', 'tauri') {
    Start-Process (Join-Path $PSScriptRoot "audio_bartender_$app.exe") -ArgumentList (@($args) + '--dev')
}
