@echo off
echo ==========================================
echo Starting Mesh Storage local test cluster
echo ==========================================
echo.
echo Starting Node 1 (API: 3000, P2P: 4001) in a new window...
start "Mesh Storage Node 1 (Port 4001)" cmd /k "node app.js -p 4001 -a 3000 -q 1.5"

echo Waiting for Node 1 to boot...
timeout /t 3 >nul

echo Starting Node 2 (API: 3001, P2P: 4002) in a new window...
start "Mesh Storage Node 2 (Port 4002)" cmd /k "node app.js -p 4002 -a 3001 -q 1.5"

echo.
echo Test cluster started!
echo Open dashboard.html in your browser and connect to:
echo - http://localhost:3000 (Node 1)
echo - http://localhost:3001 (Node 2)
echo.
pause
