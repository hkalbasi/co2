#!/usr/bin/env nu

# Build and install the CO2 VS Code extension

let script_dir = $env.PWD 
cd $script_dir

# Check vsce is installed
let has_vsce = (which vsce | length) > 0
if not $has_vsce {
    print "Installing @vscode/vsce..."
    npm install -g @vscode/vsce
}

print "Building CO2 VS Code extension..."
let result = (do -i { vsce package | complete })
if $result.exit_code != 0 {
    print -e $"Build failed:\n($result.stderr)"
    exit 1
}

let vsix_file = (ls *.vsix | sort-by modified -r | first | get name)
if ($vsix_file | is-empty) {
    print -e "No .vsix file found after packaging"
    exit 1
}

print $"Installing ($vsix_file)..."
let install_result = (do -i { code --install-extension $vsix_file | complete })
if $install_result.exit_code != 0 {
    print -e $"Installation failed:\n($install_result.stderr)"
    exit 1
}

print "CO2 extension installed successfully. Restart VS Code to activate."
