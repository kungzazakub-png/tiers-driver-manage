@echo off
rem Launcher: starts DriverScan with no console window. The script asks for administrator rights and installs itself into Program Files on first run.
powershell -WindowStyle Hidden -NoProfile -ExecutionPolicy Bypass -File "%~dp0DriverScan.ps1"
