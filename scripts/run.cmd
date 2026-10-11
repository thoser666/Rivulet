@echo off
setlocal EnableExtensions DisableDelayedExpansion

rem ============================================================================
rem CS-001 issue 3 - C55 m6/p8: auditable command wrapper for the session dir.
rem
rem A literal `cmd /c <target>` is the exact owner-controlled escape the issue
rem asks us to close, so this wrapper refuses any target outside the
rem session/worktree directory and sanitises the environment before running a
rem command. When it refuses, it still prints the full intended command block
rem under a machine-readable heading so the refusal itself is auditable rather
rem than silent.
rem
rem Usage:
rem   run.cmd command [args...]
rem
rem Environment:
rem   TARGET_DIR  directory the command runs in; default is the current
rem               directory. It must equal or live inside the session dir.
rem
rem Exit codes: 2 = missing command line, 3 = target rejected, otherwise the
rem child's own exit code.
rem ============================================================================

rem -- Session (worktree) directory: the parent directory of this script. ----
for %%I in ("%~dp0..") do set "SESSION_DIR=%%~fI"
set "SESSION_DIR=%SESSION_DIR:\=/%"
if "%SESSION_DIR:~-1%"=="/" set "SESSION_DIR=%SESSION_DIR:~0,-1%"

rem -- Target directory: explicit env override, otherwise the caller's cwd. --
if not defined TARGET_DIR set "TARGET_DIR=%CD%"
set "TARGET_DIR=%TARGET_DIR:\=/%"
if "%TARGET_DIR:~-1%"=="/" set "TARGET_DIR=%TARGET_DIR:~0,-1%"

rem -- The intended command: all arguments, verbatim. ------------------------
set "CMD_LINE=%*"
if not defined CMD_LINE (
    echo USAGE: %~nx0 command [args...]
    echo.
    echo Runs a command inside TARGET_DIR, defaulting to the current directory.
    echo The target directory must live inside the session/worktree directory.
    exit /b 2
)

rem -- Ownership check: target must equal the session dir or be nested in it. -
set "ALLOWED=0"
if /i "%TARGET_DIR%"=="%SESSION_DIR%" set "ALLOWED=1"
if "%ALLOWED%"=="0" for /f "delims=" %%I in ('echo(%TARGET_DIR% ^| findstr /i /b /l /c:"%SESSION_DIR%/"') do set "ALLOWED=1"

if "%ALLOWED%"=="0" goto :reject

rem -- Environment sanitisation. --------------------------------------------
rem Only the explicit allow-list below survives for the child process; user
rem and worktree variables such as USERPROFILE, APPDATA, RUSTUP_HOME,
rem CARGO_HOME and anything secret are deliberately NOT forwarded. The four
rem wrapper-internal variables stay defined so the run stays auditable.
for /f "delims==" %%V in ('set 2^>nul') do (
    if not "%%V"=="" if /i not "%%V"=="PATH" if /i not "%%V"=="PATHEXT" if /i not "%%V"=="SystemRoot" if /i not "%%V"=="COMSPEC" if /i not "%%V"=="TEMP" if /i not "%%V"=="TMP" if /i not "%%V"=="SESSION_DIR" if /i not "%%V"=="TARGET_DIR" if /i not "%%V"=="CMD_LINE" if /i not "%%V"=="ALLOWED" set "%%V="
)

setlocal EnableDelayedExpansion
echo [run.cmd] RUN - inside !TARGET_DIR!
echo Intended command:
echo   cmd /c !CMD_LINE!
echo.
endlocal

pushd "%TARGET_DIR%" || exit /b 1
%COMSPEC% /d /c %CMD_LINE%
set "CHILD_EXIT=%ERRORLEVEL%"
popd

echo.
echo [run.cmd] done, child exit code %CHILD_EXIT%
exit /b %CHILD_EXIT%

:reject
setlocal EnableDelayedExpansion
echo [run.cmd] REJECTED - target directory is outside the session directory.
echo.
echo SESSION_DIR : !SESSION_DIR!
echo TARGET_DIR  : !TARGET_DIR!
echo.
echo Intended command would execute:
echo.
echo   cmd /c !CMD_LINE!
echo.
echo Command not executed. No child process was started.
endlocal
exit /b 3
