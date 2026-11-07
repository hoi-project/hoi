@echo off
rem Full stack: sandbox\hoi-sandbox.cmd dashboard (Docker Desktop + Node 22).
rem Explicit session commands still route to Git Bash/WSL through the PowerShell wrapper.
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0hoi-sandbox.ps1" %*
exit /b %ERRORLEVEL%
