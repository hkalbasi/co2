#!/bin/sh

set -u

# If CO2_UPDATE_ROOT is unset or empty, default it.
CO2_UPDATE_ROOT="${CO2_UPDATE_ROOT:-https://github.com/hkalbasi/co2}"
CO2_QUIET=no
_ansi_escapes_are_valid=false
if [ -t 2 ]; then
    if [ "${TERM+set}" = 'set' ]; then
        case "$TERM" in
            xterm*|rxvt*|urxvt*|linux*|vt*)
                _ansi_escapes_are_valid=true
            ;;
        esac
    fi
fi

usage() {
    cat <<EOF
co2-install 0.1.0

The installer for co2

Usage: co2-install[EXE] [OPTIONS]

Options:
  -v, --verbose
          Set log level to 'DEBUG' if 'CO2_LOG' is unset
  -q, --quiet
          Disable progress output, set log level to 'WARN' if 'CO2_LOG' is unset
  -y, --yes
          Disable confirmation prompt
      --install-dir <INSTALL_DIR>
          Install to a specific directory [default: auto-detect]
      --dry-run
          Don't download or install, just print what would be done
  -h, --help
          Print help
  -V, --version
          Print version
EOF
}

main() {
    local need_tty=yes
    local install_dir=""

    while [ $# -gt 0 ]; do
        case "$1" in
            -h|--help)
                usage
                exit 0
                ;;
            -V|--version)
                echo "co2-install 0.1.0"
                exit 0
                ;;
            -q|--quiet)
                CO2_QUIET=yes
                ;;
            -y|--yes)
                need_tty=no
                ;;
            -v|--verbose)
                ;;
            --dry-run)
                DRY_RUN=1
                ;;
            --install-dir)
                if [ $# -lt 2 ]; then
                    err "--install-dir requires a value"
                    exit 1
                fi
                install_dir="$2"
                shift
                ;;
            --install-dir=*)
                install_dir="${1#*=}"
                ;;
            *)
                OPTIND=1
                if [ "${1%%--*}" = "" ]; then
                    warn "ignored unknown option: $1"
                else
                    while getopts :hqyvV sub_arg "$1" 2>/dev/null; do
                        case "$sub_arg" in
                            h) usage; exit 0 ;;
                            q) CO2_QUIET=yes ;;
                            y) need_tty=no ;;
                            v) ;;
                            \?) warn "ignored unknown option: -$OPTARG" ;;
                        esac
                    done
                fi
                ;;
        esac
        shift
    done

    downloader --check
    need_cmd uname
    need_cmd mktemp
    need_cmd chmod
    need_cmd mkdir
    need_cmd rm
    need_cmd rmdir

    get_architecture || return 1
    local _arch="$RETVAL"
    assert_nz "$_arch" "arch"
    check_supported "$_arch"

    local co2_version="${CO2_VERSION:-trunk}"

    if [ -z "$install_dir" ]; then
        install_dir=$(find_install_dir)
    fi

    if [ "$need_tty" = "yes" ]; then
        interactive_install co2_version install_dir
    fi

    local _url="${CO2_UPDATE_ROOT}/releases/download/${co2_version}/co2-multicall.run"
    say "installing co2 version ${co2_version}"

    if [ "${DRY_RUN-}" = "1" ]; then
        echo "DRY RUN: would download from $_url"
        echo "DRY RUN: would install to $install_dir"
        return 0
    fi

    local _dir
    if ! _dir="$(mktemp -d)"; then
        err "failed to create temporary directory"
        exit 1
    fi
    local _file="${_dir}/co2-multicall.run"

    say 'downloading co2 installer'
    ensure mkdir -p "$_dir"
    downloader "$_url" "$_file"
    ensure chmod u+x "$_file"
    if [ ! -x "$_file" ]; then
        err "Cannot execute $_file (likely because of mounting /tmp as noexec)."
        err "Please copy the file to a location where you can execute binaries and run ./co2-multicall."
        exit 1
    fi

    say "installing to $install_dir"
    local _retval=0
    if [ ! -w "$install_dir" ] 2>/dev/null; then
        say "no write permission to $install_dir, using sudo"
        ignore sudo "$_file" install "$install_dir" || _retval=$?
    else
        ignore "$_file" install "$install_dir" || _retval=$?
    fi

    ignore rm "$_file"
    ignore rmdir "$_dir"

    if [ "$_retval" -eq 0 ]; then
        say "co2 installed successfully to $install_dir"
    fi
    return "$_retval"
}

interactive_install() {
    local _version_var="$1"
    local _dir_var="$2"
    local _customized=false
    local _dir
    local _version

    eval "_dir=\$$_dir_var"
    eval "_version=\$$_version_var"

    echo ""
    echo "# Welcome to co2!"
    echo ""
    echo "This will download and install the co2 compiler system and its tools"
    echo "(co2rustc, co2cargo, co2cc, co2rustdoc, co2miri, co2fmt) into the"
    echo "following directory:"
    echo ""
    echo "    $_dir"
    echo ""

    while true; do
        echo "Current installation options:"
        echo ""
        echo "   version: $_version"
        echo "   install directory: $_dir"
        echo ""

        local _default_msg
        if [ "$_customized" = true ]; then
            _default_msg="1) Proceed with selected options (default - just press enter)"
        else
            _default_msg="1) Proceed with standard installation (default - just press enter)"
        fi
        echo "$_default_msg"
        echo "2) Customize installation"
        echo "3) Cancel installation"
        printf ">"

        local _choice
        read_input _choice

        echo ""

        case "$_choice" in
            ""|1)
                eval "$_version_var=\$_version"
                eval "$_dir_var=\$_dir"
                return 0
                ;;
            2)
                _customized=true
                customize_install _version _dir
                ;;
            *)
                echo "Installation cancelled."
                exit 0
                ;;
        esac
    done
}

customize_install() {
    local _version_var="$1"
    local _dir_var="$2"
    local _current_version
    local _current_dir

    eval "_current_version=\$$_version_var"
    eval "_current_dir=\$$_dir_var"

    echo ""
    echo "I'm going to ask you the value of each of these installation options."
    echo "You may simply press the Enter key to leave unchanged."
    echo ""

    # Version selection
    echo "Available versions (tag names):"
    echo "    trunk (latest development build)"
    # Fetch release tags from GitHub (API returns most recent first)
    local _releases
    if [ -n "${MOCK_RELEASES-}" ]; then
        _releases="$MOCK_RELEASES"
    else
        _releases=$(curl -sSf --connect-timeout 10 "https://api.github.com/repos/hkalbasi/co2/releases" 2>/dev/null) || true
    fi
    if [ -n "$_releases" ]; then
        echo "$_releases" | grep -o '"tag_name":[ ]*"[^"]*"' | cut -d'"' -f4 | grep -v '^trunk$' | head -3 | while read -r _tag; do
            echo "    $_tag"
        done
    fi
    echo "    (or any other tag - use manual input)"
    printf "Enter version tag [%s]: " "$_current_version"

    local _version_input
    read_input _version_input

    if [ -n "$_version_input" ]; then
        eval "$_version_var=\$_version_input"
    fi
    echo ""

    # Install directory
    printf "Install directory? [%s] " "$_current_dir"
    local _dir_input
    read_input _dir_input

    if [ -n "$_dir_input" ]; then
        eval "$_dir_var=\$_dir_input"
    fi
    echo ""
}

# Find a writable directory in PATH (prefer shortest)
find_install_dir() {
    local _shortest=""
    local _shortest_cargo=""

    for _dir in $(echo "$PATH" | tr ':' ' '); do
        if [ -w "$_dir" ] 2>/dev/null; then
            if [ -z "$_shortest" ] || [ ${#_dir} -lt ${#_shortest} ]; then
                _shortest="$_dir"
            fi
            case "$_dir" in
                *cargo*)
                    if [ -z "$_shortest_cargo" ] || [ ${#_dir} -lt ${#_shortest_cargo} ]; then
                        _shortest_cargo="$_dir"
                    fi
                    ;;
            esac
        fi
    done

    if [ -n "$_shortest_cargo" ]; then
        echo "$_shortest_cargo"
    elif [ -n "$_shortest" ]; then
        echo "$_shortest"
    else
        echo "/usr/local/bin"
    fi
}

check_supported() {
    local _arch="$1"

    case "$_arch" in
        x86_64-unknown-linux-gnu)
            return 0
            ;;
    esac

    echo "" >&2
    say "your platform ($_arch) is not currently supported by co2"
    say "if your platform is a tier 2 with host tools Rust target, please open an issue at:"
    say "  https://github.com/hkalbasi/co2/issues/new"
    say "  ?title=Support+for+${_arch}"
    echo "" >&2
    exit 1
}

get_architecture() {
    if [ -n "${CO2_MOCK_ARCH:-}" ]; then
        RETVAL="$CO2_MOCK_ARCH"
        return 0
    fi

    local _ostype _cputype _arch
    _ostype="$(uname -s)"
    _cputype="$(uname -m)"

    if [ "$_ostype" = Linux ]; then
        if [ "$(uname -o)" = Android ]; then
            _ostype=linux-android
        else
            _ostype=unknown-linux-gnu
        fi
    elif [ "$_ostype" = Darwin ]; then
        if [ "$_cputype" = i386 ]; then
            if (sysctl hw.optional.x86_64 2> /dev/null || true) | grep -q ': 1'; then
                _cputype=x86_64
            fi
        elif [ "$_cputype" = x86_64 ]; then
            if (sysctl hw.optional.arm64 2> /dev/null || true) | grep -q ': 1'; then
                _cputype=arm64
            fi
        fi
        _ostype=apple-darwin
    elif [ "$_ostype" = FreeBSD ]; then
        _ostype=unknown-freebsd
    elif [ "$_ostype" = NetBSD ]; then
        _ostype=unknown-netbsd
    elif [ "$_ostype" = DragonFly ]; then
        _ostype=unknown-dragonfly
    elif [ "$_ostype" = SunOS ]; then
        _ostype=pc-solaris
    elif echo "$_ostype" | grep -q 'MINGW\|MSYS\|CYGWIN\|Windows_NT'; then
        _ostype=pc-windows-gnu
    else
        err "unrecognized OS type: $_ostype"
        exit 1
    fi

    case "$_cputype" in
        i386|i486|i686|i786|x86)
            _cputype=i686
            ;;
        xscale|arm|armv6l)
            _cputype=arm
            ;;
        armv7l|armv8l)
            _cputype=armv7
            ;;
        aarch64|arm64)
            _cputype=aarch64
            ;;
        x86_64|x86-64|x64|amd64)
            _cputype=x86_64
            ;;
        ppc)
            _cputype=powerpc
            ;;
        ppc64)
            _cputype=powerpc64
            ;;
        ppc64le)
            _cputype=powerpc64le
            ;;
        s390x)
            _cputype=s390x
            ;;
        riscv64)
            _cputype=riscv64gc
            ;;
        *)
            err "unknown CPU type: $_cputype"
            exit 1
            ;;
    esac

    _arch="${_cputype}-${_ostype}"
    RETVAL="$_arch"
}

__print() {
    if $_ansi_escapes_are_valid; then
        printf '\33[1m%s:\33[0m %s\n' "$1" "$2" >&2
    else
        printf '%s: %s\n' "$1" "$2" >&2
    fi
}

warn() {
    __print 'warn' "$1" >&2
}

say() {
    if [ "$CO2_QUIET" = "no" ]; then
        __print 'info' "$1" >&2
    fi
}

err() {
    __print 'error' "$1" >&2
}

need_cmd() {
    if ! check_cmd "$1"; then
        err "need '$1' (command not found)"
        exit 1
    fi
}

check_cmd() {
    command -v "$1" > /dev/null 2>&1
}

assert_nz() {
    if [ -z "$1" ]; then
        err "assert_nz $2"
        exit 1
    fi
}

read_input() {
    if [ -t 0 ]; then
        read -r "$1"
    elif [ -n "${CO2_STDIN_FALLBACK-}" ]; then
        read -r "$1"
    else
        read -r "$1" < /dev/tty
    fi
}

ensure() {
    if ! "$@"; then
        err "command failed: $*"
        exit 1
    fi
}

ignore() {
    "$@"
}

# This wraps curl or wget. Try curl first, if not installed,
# use wget instead.
downloader() {
    local _dld
    if check_cmd curl; then
        _dld=curl
    elif check_cmd wget; then
        _dld=wget
    else
        _dld='curl or wget'
    fi

    if [ "$1" = --check ]; then
        need_cmd "$_dld"
    elif [ "$_dld" = curl ]; then
        if [ "$CO2_QUIET" = "yes" ]; then
            curl -sSfL "$1" -o "$2"
        else
            curl -fL "$1" -o "$2"
        fi
    elif [ "$_dld" = wget ]; then
        if [ "$CO2_QUIET" = "yes" ]; then
            wget -q "$1" -O "$2"
        else
            wget "$1" -O "$2"
        fi
    else
        err "Unknown downloader"
        exit 1
    fi
}

# Support for printing architecture (like rustup's RUSTUP_INIT_SH_PRINT)
set +u
case "${CO2_INIT_SH_PRINT-}" in
    arch | architecture)
        get_architecture || exit 1
        echo "$RETVAL"
        ;;
    *)
        main "$@" || exit 1
        ;;
esac
