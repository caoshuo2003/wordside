@echo off
setlocal
cd /d "%~dp0"
where node >nul 2>&1
if errorlevel 1 (
  echo Install Node.js 22 or newer, then run this file again.
  pause
  exit /b 1
)
if not exist "node_modules\express\package.json" (
  call npm exec --yes --package=pnpm@9.15.9 -- pnpm install --frozen-lockfile
  if errorlevel 1 (
    echo Dependency installation failed.
    pause
    exit /b 1
  )
)
set "HOST=127.0.0.1"
set "PORT=3000"
echo Local dictionary API: http://127.0.0.1:3000
node index.js
pause
