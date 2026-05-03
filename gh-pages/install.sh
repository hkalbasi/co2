#!/usr/bin/env sh

set -e

REPO="hkalbasi/co2"
API="https://api.github.com/repos/${REPO}"
INSTALLER="co2-multicall.run"

# Find a writable directory in PATH
find_install_dir() {
    # Prefer a path containing "cargo" if it exists and is writable
    cargo_path=""
    for dir in $(echo "$PATH" | tr ':' ' '); do
        if [ -w "$dir" ] 2>/dev/null; then
            case "$dir" in
                *cargo*) cargo_path="$dir"; break ;;
            esac
        fi
    done

    # If we found a cargo path, use it
    if [ -n "$cargo_path" ]; then
        echo "$cargo_path"
        return
    fi

    # Otherwise, find any writable path
    for dir in $(echo "$PATH" | tr ':' ' '); do
        if [ -w "$dir" ] 2>/dev/null; then
            echo "$dir"
            return
        fi
    done

    # No writable path found, ask for sudo
    echo "/usr/local/bin"
    return
}

# Get latest release download URL
get_download_url() {
    url="${API}/releases/latest"
    download_url=$(curl -s "$url" | grep -o "\"browser_download_url\": \"[^\"]*${INSTALLER}[^\"]*\"" | head -1 | cut -d'"' -f4)
    if [ -z "$download_url" ]; then
        echo "Error: Could not find ${INSTALLER} in the latest release" >&2
        exit 1
    fi
    echo "$download_url"
}

# Main installation
main() {
    echo "Finding latest release..."
    download_url=$(get_download_url)
    echo "Downloading from: $download_url"

    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' EXIT

    curl -fsSL -o "${tmpdir}/${INSTALLER}" "$download_url"
    chmod +x "${tmpdir}/${INSTALLER}"

    install_dir=$(find_install_dir)
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
