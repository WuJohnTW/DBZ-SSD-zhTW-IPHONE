# Dot-source this before building on Windows, from any folder:
#
#     . tools\no-local-paths.ps1
#     cargo build --release -p ssd-app
#
# The exe then names no folder of the machine that built it. Rust's panic locations name cargo's home as `/cargo`,
# the repository as `.` and the user's folder as `~`. With an MSVC target, the C sources' __FILE__ (SDL, which the
# sdl3-sys crate builds through cmake and the cc crate) lose those folders' prefixes (cl.exe /d1trimfile) and the
# PDB is named by its file name only (link.exe /PDBALTPATH:%_PDB%).
#
# How to check a built exe (each count is expected to be 0; the search ignores case):
#
#     $exe = "$PWD\target\release\ssd-recompiled.exe"
#     $b = [IO.File]::ReadAllText($exe, [Text.Encoding]::GetEncoding(28591))
#     $u16 = [Text.Encoding]::Unicode.GetString([IO.File]::ReadAllBytes($exe))
#     $repoName = Split-Path -Leaf (Resolve-Path "$PSScriptRoot\..")      # or the repository folder's name
#     foreach ($p in @($env:USERPROFILE, ($env:USERPROFILE -replace '\\', '/'), '.cargo\registry', '.cargo/registry',
#                      $env:USERNAME, $repoName)) {
#         $n = [regex]::Matches($b, [regex]::Escape($p), 'IgnoreCase').Count +
#              [regex]::Matches($u16, [regex]::Escape($p), 'IgnoreCase').Count
#         "{0,-40} {1}" -f $p, $n
#     }
#     dumpbin /headers $exe | Select-String -Pattern '\.pdb'      # the PDB: a file name, no folder
#
# (A user name or folder name that is also an ordinary word may match text that is not a path: look at the hits.)
#
# It sets CARGO_ENCODED_RUSTFLAGS (which takes the place of RUSTFLAGS and of the rustflags of .cargo/config.toml, so
# the /PDBALTPATH of that file is passed here too) and adds to CFLAGS and CXXFLAGS (which the cc crate passes to the
# compiler, and the cmake crate on to cmake). Dot-sourcing it again gives the same values, not doubled ones.
# Folders whose names have spaces cannot be passed in CFLAGS: the script stops and says which one.
# /d1trimfile is an undocumented cl.exe option (Visual Studio 2019 16.x and later).

$nlpRepo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path.TrimEnd('\')
$nlpCargo = if ($env:CARGO_HOME) { $env:CARGO_HOME.TrimEnd('\') } else { Join-Path $env:USERPROFILE '.cargo' }
$nlpUser = $env:USERPROFILE.TrimEnd('\')
foreach ($nlpPair in @(@('USERPROFILE', $nlpUser), @('CARGO_HOME', $nlpCargo), @('the repository', $nlpRepo))) {
    if ($nlpPair[1] -match '\s') {
        throw ("no-local-paths: $($nlpPair[0]) has a space in it ('$($nlpPair[1])'). The compiler flags cannot " +
               "carry such a folder: build from folders without spaces (set CARGO_HOME, or clone the repository " +
               "into a folder such as C:\src\DBZ-SSD-zhTW).")
    }
}

# The target: CARGO_BUILD_TARGET if set, else the host rustc builds for; MSVC unless either says otherwise
$nlpTarget = $env:CARGO_BUILD_TARGET
if (-not $nlpTarget) {
    try {
        $nlpHost = & rustc -vV 2>$null | Where-Object { $_ -like 'host: *' } | Select-Object -First 1
        if ($nlpHost) { $nlpTarget = $nlpHost.Substring(6).Trim() }
    } catch { $nlpTarget = $null }
}
$nlpMsvc = (-not $nlpTarget) -or ($nlpTarget -like '*-msvc')

# rustc applies the last of the --remap-path-prefix that match: the most specific folders go last
$nlpRust = @(
    "--remap-path-prefix=$nlpUser=~",
    "--remap-path-prefix=$nlpCargo=/cargo",
    "--remap-path-prefix=$nlpRepo=."
)
if ($nlpMsvc) {
    $nlpRust += '-Clink-arg=/PDBALTPATH:%_PDB%'
    $nlpC = "/d1trimfile:$nlpCargo /d1trimfile:$nlpRepo /d1trimfile:$nlpUser"
} else {
    $nlpC = "-ffile-prefix-map=$nlpUser=~ -ffile-prefix-map=$nlpCargo=/cargo -ffile-prefix-map=$nlpRepo=."
}
$env:CARGO_ENCODED_RUSTFLAGS = $nlpRust -join [char]0x1f

# CFLAGS / CXXFLAGS: what they were before the first dot-source, then these flags (so a second one adds nothing)
if ($env:DBZ_NO_LOCAL_PATHS -ne '1') {
    $env:DBZ_NLP_CFLAGS = $env:CFLAGS
    $env:DBZ_NLP_CXXFLAGS = $env:CXXFLAGS
    $env:DBZ_NO_LOCAL_PATHS = '1'
}
$env:CFLAGS = (@($env:DBZ_NLP_CFLAGS, $nlpC) | Where-Object { $_ }) -join ' '
$env:CXXFLAGS = (@($env:DBZ_NLP_CXXFLAGS, $nlpC) | Where-Object { $_ }) -join ' '

Remove-Variable nlpRepo, nlpCargo, nlpUser, nlpPair, nlpTarget, nlpMsvc, nlpRust, nlpC -ErrorAction SilentlyContinue
Remove-Variable nlpHost -ErrorAction SilentlyContinue
