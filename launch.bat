@echo off
setlocal enabledelayedexpansion
title Mesh Storage Network — Live LAN Launcher
color 0B

:: Ensure working directory is the script directory
cd /d "%~dp0"

echo ======================================================================
echo           MESH STORAGE NETWORK - DECENTRALIZED P2P CLOUD
echo ======================================================================
echo.

:: Detect local LAN IPv4 address using PowerShell
echo Detecting local network interface...
set "LAN_IP="
for /f "usebackq tokens=*" %%i in (`powershell -NoProfile -Command "(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -notmatch 'Loopback|vEthernet|Virtual|WSL|Tailscale' -and $_.IPAddress -notmatch '^169\.' -and $_.IPAddress -notmatch '^127\.' } | Select-Object -First 1).IPAddress"`) do set "LAN_IP=%%i"

if "%LAN_IP%"=="" (
    set "LAN_IP=127.0.0.1"
    echo [WARNING] No external LAN IP detected. Falling back to 127.0.0.1
) else (
    echo [OK] Detected LAN IPv4 Address: %LAN_IP%
)

echo.
echo ======================================================================
echo   [1] Start Anchor Node (Node 1) on Port 4001 ^& Open Dashboard  (Recommended)
echo   [2] Start Second Peer Node (Node 2) on Port 4002 / API 3001
echo   [3] Open Visual Dashboard in Default Browser
echo   [4] Run Full Workspace Test Suite (cargo test --workspace)
echo   [5] Exit
echo ======================================================================
echo.

set /p "CHOICE=Select option [1-5] (default is 1): "
if "%CHOICE%"=="" set "CHOICE=1"

if "%CHOICE%"=="1" goto start_node1
if "%CHOICE%"=="2" goto start_node2
if "%CHOICE%"=="3" goto open_browser
if "%CHOICE%"=="4" goto run_tests
if "%CHOICE%"=="5" goto end
goto start_node1

:start_node1
echo.
echo ======================================================================
echo  LAUNCHING ANCHOR NODE (NODE 1) ACROSS LAN
echo ======================================================================
echo  * Local Dashboard:    http://localhost:3000/
echo  * LAN Mobile URL:     http://%LAN_IP%:3000/
echo  * P2P Swarm Port:     4001
echo  * HTTP API Port:      3000
echo  * Security API Key:   mesh_secret_lan_key_987
echo ======================================================================
echo.
echo [INFO] Opening Web Dashboard in your default browser...
start "" "http://localhost:3000/"

:: Set environment variables for LAN mode and mandatory authentication
set "MESH_API_KEY=mesh_secret_lan_key_987"
set "MESH_ENV=lan"
set "CARGO_TARGET_DIR=%USERPROFILE%\.gemini\antigravity-ide\scratch\cargo-target"

echo [INFO] Starting mesh-node daemon (Press Ctrl+C to stop)...
cargo run -p mesh-node -- --bind 0.0.0.0 --port 4001 --api-port 3000 --quota 5.0
goto end

:start_node2
echo.
echo ======================================================================
echo  LAUNCHING SECOND PEER NODE (NODE 2)
echo ======================================================================
echo  * Local Dashboard:    http://localhost:3001/
echo  * LAN Mobile URL:     http://%LAN_IP%:3001/
echo  * P2P Swarm Port:     4002
echo  * HTTP API Port:      3001
echo  * Security API Key:   mesh_secret_lan_key_987
echo ======================================================================
echo.
echo [INFO] Opening Second Node Dashboard in browser...
start "" "http://localhost:3001/"

set "MESH_API_KEY=mesh_secret_lan_key_987"
set "MESH_ENV=lan"
set "CARGO_TARGET_DIR=%USERPROFILE%\.gemini\antigravity-ide\scratch\cargo-target"

echo [INFO] Starting secondary mesh-node daemon (Press Ctrl+C to stop)...
cargo run -p mesh-node -- --bind 0.0.0.0 --port 4002 --api-port 3001 --quota 5.0
goto end

:open_browser
echo.
echo [INFO] Opening http://localhost:3000/ in browser...
start "" "http://localhost:3000/"
goto end

:run_tests
echo.
echo ======================================================================
echo  RUNNING FULL WORKSPACE AUTOMATED VERIFICATION SUITE
echo ======================================================================
set "CARGO_TARGET_DIR=%USERPROFILE%\.gemini\antigravity-ide\scratch\cargo-target"
cargo test --workspace
echo.
pause
goto end

:end
