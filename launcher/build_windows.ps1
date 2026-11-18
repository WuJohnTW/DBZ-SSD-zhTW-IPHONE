# Build the launcher on any Windows machine with a Rust toolchain and Visual Studio's C++ build tools (rc.exe makes
# the version resource; vcvars64 is loaded first). cargo runs with -j 1 at BelowNormal priority, to keep a shared
# machine responsive; drop both if the machine is yours alone.
# Usage (from this folder): powershell -NoProfile -ExecutionPolicy Bypass -File build_windows.ps1 [check|build] [-CargoBin <dir>]
#   -CargoBin: the folder with cargo.exe (default: %USERPROFILE%\.cargo\bin, rustup's).
param([string]$What = 'build', [string]$CargoBin = (Join-Path $env:USERPROFILE '.cargo\bin'))
$ErrorActionPreference = 'Continue'
$env:Path = "$CargoBin;" + $env:Path
$vs = & (Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe') -products * -latest -property installationPath
cmd /c "`"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul && set" | ForEach-Object { if ($_ -match '^(.*?)=(.*)$') { Set-Item "env:$($matches[1])" $matches[2] } }
Set-Location $PSScriptRoot
. (Join-Path $PSScriptRoot '..\tools\no-local-paths.ps1')  # no folder of this machine in the exe
$err = Join-Path $PSScriptRoot "$What.err"
$cargoArgs = if ($What -eq 'check') { @('check', '-j', '1') } else { @('build', '--release', '-j', '1') }
$t0 = Get-Date
$p = Start-Process cargo -ArgumentList $cargoArgs -NoNewWindow -PassThru -RedirectStandardError $err
$null = $p.Handle  # keeps ExitCode readable
try { $p.PriorityClass = 'BelowNormal' } catch {}
$p.WaitForExit()
$secs = [int]((Get-Date) - $t0).TotalSeconds
Get-Content $err -Tail 30
"exit=$($p.ExitCode) seconds=$secs"
