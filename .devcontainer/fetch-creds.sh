#!/bin/bash

if [[ "$OSTYPE" == "darwin"* ]]; then
    # macOS: Extract Keychain → bridge file
    creds_file="${XDG_DATA_HOME:-$HOME/.local/share}/claude-devcontainer/credentials.json"
    echo "Fetching Claude credentials from macOS Keychain to $creds_file"
    mkdir -p "$(dirname "$creds_file")"
    security find-generic-password -s "Claude Code-credentials" -a "$USER" -w > "$creds_file"
fi