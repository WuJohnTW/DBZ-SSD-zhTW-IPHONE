# Source this (bash or zsh) before building on macOS or Linux, from any folder:
#
#     . tools/no-local-paths.sh
#     cargo build --release -p ssd-app
#
# The binaries then name no folder of the machine that built them. Rust's panic locations and the C sources'
# __FILE__ (SDL, which the sdl3-sys crate builds through cmake and the cc crate) name cargo's home as `/cargo`, the
# repository as `.` and the home folder as `~`. Check a binary with (each count is expected to be 0):
#
#     for p in "$HOME" .cargo/registry "$USER" "$(basename "$(git rev-parse --show-toplevel)")"; do
#         printf '%-40s %s\n' "$p" "$(strings target/release/ssd-recompiled | grep -ci -- "$p")"
#     done
#
# (A user name or folder name that is also an ordinary word may match text that is not a path: look at the hits.)
#
# It sets CARGO_ENCODED_RUSTFLAGS (which takes the place of RUSTFLAGS and of the rustflags of .cargo/config.toml) and
# adds to CFLAGS and CXXFLAGS (which the cc crate passes to the compiler, and the cmake crate on to cmake). Sourcing
# it again gives the same values, not doubled ones. Folders whose names have spaces cannot be passed in CFLAGS: it
# stops and says which one.

# The repository: the folder above this script (bash: BASH_SOURCE; zsh: %x), else git's top level, else here
if [ -n "${BASH_SOURCE:-}" ]; then
    _nlp_self=${BASH_SOURCE[0]}
elif [ -n "${ZSH_VERSION:-}" ]; then
    eval '_nlp_self=${(%):-%x}'
else
    _nlp_self=""
fi
if [ -n "$_nlp_self" ]; then
    _nlp_repo=$(cd "$(dirname "$_nlp_self")/.." && pwd -P)
else
    _nlp_repo=$(git rev-parse --show-toplevel 2>/dev/null || pwd -P)
fi
_nlp_cargo=${CARGO_HOME:-$HOME/.cargo}
_nlp_ok=1
for _nlp_pair in "HOME=$HOME" "CARGO_HOME=$_nlp_cargo" "the repository=$_nlp_repo"; do
    case "${_nlp_pair#*=}" in
        *[[:space:]]*)
            echo "no-local-paths: ${_nlp_pair%%=*} has a space in it ('${_nlp_pair#*=}'): build from folders without spaces." >&2
            _nlp_ok=0 ;;
    esac
done
if [ "$_nlp_ok" = 1 ]; then
    # rustc applies the last of the --remap-path-prefix that match: the most specific folders go last
    _nlp_flags=""
    for _nlp_map in "$HOME=~" "$_nlp_cargo=/cargo" "$_nlp_repo=."; do
        _nlp_flags="${_nlp_flags:+$_nlp_flags$(printf '\037')}--remap-path-prefix=$_nlp_map"
    done
    export CARGO_ENCODED_RUSTFLAGS="$_nlp_flags"
    # gcc and clang take the longest prefix that matches. CFLAGS / CXXFLAGS: as they were before the first source,
    # then these flags, so that sourcing again adds nothing
    if [ "${DBZ_NO_LOCAL_PATHS:-}" != 1 ]; then
        export DBZ_NLP_CFLAGS="${CFLAGS:-}" DBZ_NLP_CXXFLAGS="${CXXFLAGS:-}" DBZ_NO_LOCAL_PATHS=1
    fi
    _nlp_c="-ffile-prefix-map=$HOME=~ -ffile-prefix-map=$_nlp_cargo=/cargo -ffile-prefix-map=$_nlp_repo=."
    export CFLAGS="${DBZ_NLP_CFLAGS:+$DBZ_NLP_CFLAGS }$_nlp_c"
    export CXXFLAGS="${DBZ_NLP_CXXFLAGS:+$DBZ_NLP_CXXFLAGS }$_nlp_c"
fi
_nlp_rc=$((1 - _nlp_ok))
unset _nlp_self _nlp_repo _nlp_cargo _nlp_ok _nlp_pair _nlp_flags _nlp_map _nlp_c
return $_nlp_rc 2>/dev/null || exit $_nlp_rc
