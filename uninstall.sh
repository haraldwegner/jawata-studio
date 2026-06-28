#!/usr/bin/env bash

APP_NAME="goja-studio"
BIN_DIR="$HOME/.local/bin"
APP_DIR="$HOME/.local/share/applications"
ICON_DIR="$HOME/.local/share/icons/hicolor/128x128/apps"

echo "Uninstalling $APP_NAME..."

if [ -f "$BIN_DIR/$APP_NAME" ]; then
    rm "$BIN_DIR/$APP_NAME"
    echo "Removed executable from $BIN_DIR"
fi

if [ -f "$APP_DIR/$APP_NAME.desktop" ]; then
    rm "$APP_DIR/$APP_NAME.desktop"
    echo "Removed desktop entry from $APP_DIR"
fi

if [ -f "$ICON_DIR/$APP_NAME.png" ]; then
    rm "$ICON_DIR/$APP_NAME.png"
    echo "Removed icon from $ICON_DIR"
fi

echo "Uninstallation complete. Note: Configuration and data files in ~/.config/goja-studio and ~/.local/state/goja-studio were kept."
