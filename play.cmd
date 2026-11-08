@echo off
rem Runs play.ps1 even where PowerShell scripts are blocked; same options.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0play.ps1" %*
exit /b %ERRORLEVEL%
