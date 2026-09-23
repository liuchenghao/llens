# LLens - force refresh desktop shortcut icon to the new app icon.
# Run after rebuilding. Steps:
#  1. (optional) verify the target exe carries the new icon
#  2. clear Windows icon cache
#  3. delete the old desktop shortcut so NSIS re-creates it on next install
param(
  [string]$Exe = "src-tauri\target\release\llens.exe",
  [string]$ShortcutName = "llens.lnk"
)
Write-Host "[1/3] Checking exe icon hash (informational)..." -ForegroundColor Cyan
if (Test-Path $Exe) {
  Get-FileHash $Exe -Algorithm MD5 | Select-Object Hash
} else {
  Write-Host "  exe not found at $Exe (build first)" -ForegroundColor Yellow
}

Write-Host "[2/3] Clearing Windows icon cache..." -ForegroundColor Cyan
$cache = Join-Path $env:LOCALAPPDATA "Microsoft\Windows\Explorer"
Get-ChildItem -Path $cache -Filter "iconcache*" -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
Get-ChildItem -Path $cache -Filter "thumbcache*" -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
# Restart Explorer to drop in-memory icon cache
Stop-Process -Name explorer -Force -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1
Start-Process explorer
Write-Host "  Icon cache cleared, Explorer restarted." -ForegroundColor Green

Write-Host "[3/3] Removing stale desktop shortcut (NSIS will recreate it on next install)..." -ForegroundColor Cyan
$desk = Join-Path ([Environment]::GetFolderPath("Desktop")) $ShortcutName
if (Test-Path $desk) {
  Remove-Item $desk -Force
  Write-Host "  Removed $desk" -ForegroundColor Green
} else {
  Write-Host "  $ShortcutName not found on desktop (already clean)" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "Next: re-run the NSIS installer (check 'create desktop shortcut')." -ForegroundColor Green
Write-Host "The rebuilt .lnk will pick up the new exe icon after the cache clears above." -ForegroundColor Green
