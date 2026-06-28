#!/usr/bin/env bash
set -e

REPO="haraldwegner/goja-studio"
APP_NAME="goja-studio"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/128x128/apps"

# Sprint 14 (v0.14.0): macOS install path. Mirrors the Linux flow's
# shape but uses hdiutil + cp to /Applications + xattr (Gatekeeper
# bypass for unsigned). The Linux flow below this function is unchanged.
install_macos() {
    local mac_arch dmg_arch dmg_url tmp_dmg mount_dir app_src app_name

    mac_arch=$(uname -m)
    case "$mac_arch" in
        arm64)   dmg_arch="aarch64" ;;
        x86_64)
            echo "Error: Intel Macs are not supported."
            echo "Apple stopped shipping Intel Macs in 2023; remaining hardware is six-plus years old."
            echo "Workaround: install Rosetta 2 (softwareupdate --install-rosetta) and run the Apple Silicon .dmg via translation."
            exit 1
            ;;
        *)
            echo "Error: unsupported macOS arch '$mac_arch'."
            echo "Supported: arm64 (Apple Silicon)."
            exit 1
            ;;
    esac
    echo "Detected macOS arch: $mac_arch -> $dmg_arch"

    echo "Fetching latest release from GitHub..."
    dmg_url=$(curl -sSL "https://api.github.com/repos/$REPO/releases/latest" \
        | grep -oE '"browser_download_url": "[^"]*_'"$dmg_arch"'\.dmg"' \
        | sed -E 's/.*"browser_download_url": "([^"]+)".*/\1/' \
        | head -1)

    if [ -z "$dmg_url" ]; then
        echo "Error: Could not find a .dmg for arch '$dmg_arch' in the latest release."
        echo "Check https://github.com/$REPO/releases/latest for available assets."
        exit 1
    fi

    tmp_dmg="/tmp/$APP_NAME.$$.dmg"
    mount_dir="/tmp/$APP_NAME-install.$$"

    trap 'hdiutil detach -quiet "$mount_dir" 2>/dev/null || true; rm -rf "$tmp_dmg" "$mount_dir"' EXIT

    echo "Downloading $APP_NAME..."
    curl -sSL -o "$tmp_dmg" "$dmg_url"

    echo "Mounting DMG..."
    mkdir -p "$mount_dir"
    hdiutil attach -nobrowse -quiet -mountpoint "$mount_dir" "$tmp_dmg"

    # Find the .app bundle in the mounted volume. Tauri's DMG typically
    # contains exactly one .app at the top level.
    app_src=$(ls -d "$mount_dir"/*.app 2>/dev/null | head -1)
    if [ -z "$app_src" ] || [ ! -d "$app_src" ]; then
        echo "Error: no .app bundle found in the DMG"
        exit 1
    fi
    app_name=$(basename "$app_src")

    echo "Copying $app_name to /Applications..."
    rm -rf "/Applications/$app_name"
    cp -R "$app_src" /Applications/

    # Sprint 14 (v0.14.0): unsigned in v0.14.0; strip the Gatekeeper
    # quarantine attribute so the user doesn't have to right-click →
    # Open the first time. Apple Developer signing is a separate
    # later track.
    xattr -d com.apple.quarantine "/Applications/$app_name" 2>/dev/null || true

    echo "Installation complete! Launch '$app_name' from /Applications or Launchpad."
}

# Sprint 14 (v0.14.0): OS dispatcher. Darwin handled by install_macos;
# Linux falls through to the existing AppImage flow below.
case "$(uname -s)" in
    Darwin)
        install_macos
        exit 0
        ;;
    Linux)
        ;; # fall through to the Linux flow
    *)
        echo "Error: unsupported OS '$(uname -s)'."
        echo "Supported: Linux (Ubuntu / Debian / Fedora / etc) and Darwin (macOS)."
        exit 1
        ;;
esac

MACHINE=$(uname -m)
case "$MACHINE" in
    x86_64)         APPIMAGE_ARCH="amd64" ;;
    aarch64|arm64)  APPIMAGE_ARCH="aarch64" ;;
    *)
        echo "Error: unsupported architecture '$MACHINE'."
        echo "Supported: x86_64 (amd64) and aarch64/arm64."
        exit 1
        ;;
esac
echo "Detected architecture: $MACHINE -> $APPIMAGE_ARCH"

echo "Fetching latest release from GitHub..."
LATEST_RELEASE=$(curl -sSL "https://api.github.com/repos/$REPO/releases/latest")
APPIMAGE_URL=$(echo "$LATEST_RELEASE" | grep -oP "\"browser_download_url\": \"\K(.*_${APPIMAGE_ARCH}\.AppImage)(?=\")")

if [ -z "$APPIMAGE_URL" ]; then
    echo "Error: Could not find an .AppImage for architecture '$APPIMAGE_ARCH' in the latest release."
    echo "Check https://github.com/$REPO/releases/latest for available assets."
    exit 1
fi

# Sprint 14 (v0.14.0, bugs.md #1 full fix): the v0.14.0+ AppImage bakes
# WEBKIT_DISABLE_DMABUF_RENDERER=1 into its own AppRun, so the wrapper
# script that v0.13.1 install.sh wrote at $BIN_DIR/$APP_NAME is no
# longer needed — the AppImage IS the binary. The desktop entry now
# launches the AppImage directly.
echo "Downloading $APP_NAME..."
mkdir -p "$BIN_DIR"
curl -sSL -o "$BIN_DIR/$APP_NAME" "$APPIMAGE_URL"
chmod +x "$BIN_DIR/$APP_NAME"

# Clean up the legacy AppImage file from pre-v0.14.0 installs, where
# install.sh wrote $BIN_DIR/$APP_NAME as a wrapper and the actual
# AppImage as $BIN_DIR/$APP_NAME.AppImage. Harmless if not present.
rm -f "$BIN_DIR/$APP_NAME.AppImage"

echo "Setting up desktop entry..."
mkdir -p "$APP_DIR"
mkdir -p "$ICON_DIR"

# Download icon from the repository
curl -sSL -o "$ICON_DIR/$APP_NAME.png" "https://raw.githubusercontent.com/$REPO/main/src-tauri/icons/128x128.png"

cat > "$APP_DIR/$APP_NAME.desktop" <<EOF
[Desktop Entry]
Name=goja-studio
Exec=$BIN_DIR/$APP_NAME
Icon=$ICON_DIR/$APP_NAME.png
Type=Application
Categories=Development;
Terminal=false
EOF

# Try to update desktop database and icon cache silently if the tools are available
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APP_DIR" || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -f -t "$HOME/.local/share/icons/hicolor" || true
fi

echo "Installation complete! You can now launch $APP_NAME from your application menu."
echo "Note: Make sure $BIN_DIR is in your PATH."
