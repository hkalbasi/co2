#!/usr/bin/env sh

set -e

REPO="hkalbasi/co2"
API="https://api.github.com/repos/${REPO}"
INSTALLER="co2-multicall.run"

# Find a writable directory in PATH (prefer shortest)
find_install_dir() {
    shortest=""
    shortest_cargo=""

    for dir in $(echo "$PATH" | tr ':' ' '); do
        if [ -w "$dir" ] 2>/dev/null; then
            # Track shortest writable path
            if [ -z "$shortest" ] || [ ${#dir} -lt ${#shortest} ]; then
                shortest="$dir"
            fi
            # Track shortest cargo path
            case "$dir" in
                *cargo*)
                    if [ -z "$shortest_cargo" ] || [ ${#dir} -lt ${#shortest_cargo} ]; then
                        shortest_cargo="$dir"
                    fi
                    ;;
            esac
        fi
    done

    # Prefer shortest cargo path, fallback to shortest writable path
    if [ -n "$shortest_cargo" ]; then
        echo "$shortest_cargo"
    elif [ -n "$shortest" ]; then
        echo "$shortest"
    else
        echo "/usr/local/bin"
    fi
}

# Get latest release info
get_release_info() {
    url="${API}/releases/latest"

    # Fetch release info with error handling (disable set -e temporarily)
    response=$(curl -s --connect-timeout 10 "$url" 2>&1) || true
    if [ -z "$response" ]; then
        echo "Error: Failed to fetch release info from GitHub API" >&2
        echo "Please check your internet connection and try again." >&2
        exit 1
    fi

    # Check if API returned an error
    if echo "$response" | grep -q '"message":'; then
        api_error=$(echo "$response" | grep -o '"message": "[^"]*"' | head -1 | cut -d'"' -f4)
        echo "Error: GitHub API returned an error: $api_error" >&2
        exit 1
    fi

    # Get download URL
    download_url=$(echo "$response" | grep -o "\"browser_download_url\": \"[^\"]*${INSTALLER}[^\"]*\"" | head -1 | cut -d'"' -f4)
    if [ -z "$download_url" ]; then
        echo "Error: Could not find ${INSTALLER} in the latest release" >&2
        exit 1
    fi

    # Get version
    version=$(echo "$response" | grep -o '"tag_name": "[^"]*"' | head -1 | cut -d'"' -f4)
    if [ -z "$version" ]; then
        version="unknown"
    fi

    # Get file size using the release assets API
    asset_url=$(echo "$response" | grep -o "\"url\": \"[^\"]*/assets/[0-9]*\"" | head -1 | cut -d'"' -f4)
    if [ -n "$asset_url" ]; then
        size_bytes=$(curl -s "$asset_url" | grep -o '"size": [0-9]*' | head -1 | grep -o '[0-9]\+' || true)
    fi

    if [ -n "$size_bytes" ]; then
        size_mb=$(( (size_bytes + 524288) / 1048576 ))
        size_str="${size_mb} MB"
    else
        size_str="unknown size"
    fi

    echo "$download_url|$version|$size_str"
}

# Ask for confirmation
confirm_install() {
    download_url="$1"
    version="$2"
    size="$3"
    install_dir="$4"

    echo ""
    echo "========================================"
    echo "  CO2 Installation Summary"
    echo "========================================"
    echo "  Version:    $version"
    echo "  Size:       $size"
    echo "  Install to: $install_dir"
    echo "========================================"
    echo ""
    printf "Continue? [y/N] "

    # Read from terminal directly (needed when piped via curl | sh)
    if [ -t 0 ]; then
        read -r answer
    else
        read -r answer < /dev/tty
    fi

    case "$answer" in
        [Yy]|[Yy][Ee][Ss]) return 0 ;;
        *) echo "Installation cancelled."; exit 0 ;;
    esac
}

# Main installation
main() {
    echo "Finding latest release..."

    info=$(get_release_info)
    download_url=$(echo "$info" | cut -d'|' -f1)
    version=$(echo "$info" | cut -d'|' -f2)
    size=$(echo "$info" | cut -d'|' -f3)

    install_dir=$(find_install_dir)

    confirm_install "$download_url" "$version" "$size" "$install_dir"

    echo "Downloading from: $download_url"

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    curl -fsSL -o "${tmpdir}/${INSTALLER}" "$download_url"
    chmod +x "${tmpdir}/${INSTALLER}"

    echo "Install directory: $install_dir"

    if [ ! -w "$install_dir" ] 2>/dev/null; then
        echo "No writable path found in PATH, using sudo to install to $install_dir"
        sudo "${tmpdir}/${INSTALLER}" install "$install_dir"
    else
        "${tmpdir}/${INSTALLER}" install "$install_dir"
    fi

    echo "CO2 installed successfully!"
}

main "$@"
