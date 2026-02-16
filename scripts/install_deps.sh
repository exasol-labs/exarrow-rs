#!/bin/bash
# Install prerequisites for building and testing exarrow-rs.
# Supports Ubuntu (EC2 target) and macOS.
#
# Usage: ./scripts/install_deps.sh
set -euo pipefail

OS="$(uname -s)"

echo "Detected OS: $OS"

case "$OS" in
  Linux)
    if ! command -v apt-get >/dev/null 2>&1; then
      echo "Only Ubuntu/Debian (apt-based) is supported."
      exit 1
    fi

    echo "--- Installing system packages ---"
    sudo apt-get update
    sudo apt-get install -y \
      build-essential \
      docker.io \
      python3 \
      python3-pip \
      pkg-config \
      libssl-dev \
      cmake

    # Add current user to docker group if not already
    if ! groups | grep -q docker; then
      sudo usermod -aG docker "$USER"
      echo "Added $USER to docker group. You may need to log out and back in."
    fi

    # Install Rust if not present
    if ! command -v rustup >/dev/null 2>&1; then
      echo "--- Installing Rust ---"
      curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
      # shellcheck source=/dev/null
      source "$HOME/.cargo/env"
    else
      echo "Rust already installed: $(rustc --version)"
      rustup update stable
    fi

    # Install cargo-deny
    if ! command -v cargo-deny >/dev/null 2>&1; then
      echo "--- Installing cargo-deny ---"
      cargo install cargo-deny
    else
      echo "cargo-deny already installed"
    fi
    ;;

  Darwin)
    echo "--- Checking macOS dependencies ---"
    local_missing=()

    if ! command -v brew >/dev/null 2>&1; then
      echo "Homebrew not found. Install from https://brew.sh"
      local_missing+=("homebrew")
    fi

    if ! command -v docker >/dev/null 2>&1; then
      echo "Docker not found. Install Docker Desktop from https://www.docker.com/products/docker-desktop/"
      local_missing+=("docker")
    fi

    if ! command -v rustup >/dev/null 2>&1; then
      echo "Rust not found."
      echo "  Install: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
      local_missing+=("rust")
    else
      echo "Rust: $(rustc --version)"
    fi

    if ! command -v python3 >/dev/null 2>&1; then
      echo "Python3 not found."
      echo "  Install: brew install python3"
      local_missing+=("python3")
    else
      echo "Python: $(python3 --version)"
    fi

    if ! command -v cargo-deny >/dev/null 2>&1; then
      echo "cargo-deny not found."
      echo "  Install: cargo install cargo-deny"
      local_missing+=("cargo-deny")
    else
      echo "cargo-deny: installed"
    fi

    if [[ ${#local_missing[@]} -gt 0 ]]; then
      echo ""
      echo "Missing tools: ${local_missing[*]}"
      echo "Install the above and re-run this script."
      exit 1
    fi

    echo "All dependencies present."
    ;;

  *)
    echo "Unsupported OS: $OS"
    exit 1
    ;;
esac

echo ""
echo "Done. Run ./scripts/run_all_tests.sh to execute the full test suite."
