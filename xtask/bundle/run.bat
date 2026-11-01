@echo off
setlocal
cd /d "%~dp0"
title Voxel game test
echo.
echo This test takes about 10 minutes. Please leave this window open until it says Done.
echo.
set REPORT=voxel-report.txt
echo Voxel game test report> %REPORT%
echo.>> %REPORT%
echo === Computer ===>> %REPORT%
powershell -NoProfile -Command "Get-CimInstance Win32_OperatingSystem | Select-Object Caption,Version,TotalVisibleMemorySize | Format-List; Get-CimInstance Win32_Processor | Select-Object Name,NumberOfCores,NumberOfLogicalProcessors | Format-List; Get-CimInstance Win32_VideoController | Select-Object Name,DriverVersion,CurrentHorizontalResolution,CurrentVerticalResolution,CurrentRefreshRate | Format-List; Get-CimInstance Win32_ComputerSystem | Select-Object Manufacturer,Model | Format-List" >> %REPORT% 2>&1
for %%P in (spike-gpu spike-wasmtime spike-mesher spike-redb spike-circuit spike-quinn) do call :run %%P
echo.
echo Done. Please send the file voxel-report.txt from this folder to whoever sent you this.
echo.
pause
exit /b 0

:run
echo Running %1 ...
echo.>> %REPORT%
echo === %1 ===>> %REPORT%
%1.exe >> %REPORT% 2>&1
echo exit code: %ERRORLEVEL%>> %REPORT%
exit /b 0
