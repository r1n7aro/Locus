#!/usr/bin/env bash
# Use the isolated Linux/WSL toolchain; never override the Windows toolchain.
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
component="${1:-app}"
architecture="${2:-all}"
mode="${3:-check}"
case "$component" in
  app) manifest="$repo_root/src-tauri/Cargo.toml" ;;
  native) manifest="$repo_root/locus_native_plugin/Cargo.toml" ;;
  *) printf '%s\n' 'Usage: check-macos-cross.sh app|native arm64|x64|all check|build [--release]' >&2; exit 2 ;;
esac
case "$architecture" in
  arm64) targets=(aarch64-apple-darwin) ;;
  x64) targets=(x86_64-apple-darwin) ;;
  all) targets=(aarch64-apple-darwin x86_64-apple-darwin) ;;
  *) printf 'Unsupported architecture: %s\n' "$architecture" >&2; exit 2 ;;
esac
case "$mode" in check|build) ;; *) exit 2 ;; esac
profile_args=()
case "${4:-}" in '') ;; --release) profile_args+=(--release) ;; *) exit 2 ;; esac

cross_root="${LOCUS_MACOS_CROSS_ROOT:-/opt/locus-macos}"
export SDKROOT="${SDKROOT:-$cross_root/sdk/SDKs/MacOSX14.5.sdk}"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export XMAC_CLANG="${XMAC_CLANG:-/usr/bin/clang-19}"
export XMAC_CLANGXX="${XMAC_CLANGXX:-/usr/bin/clang++-19}"
export XMAC_LLD="${XMAC_LLD:-$cross_root/bin/ld64.lld}"
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$cross_root/bin:$PATH"
toolchain="${LOCUS_MACOS_RUST_TOOLCHAIN:-1.95.0}"
for required in "$SDKROOT/SDKSettings.json" "$XMAC_CLANG" "$XMAC_LLD"; do
  if [[ ! -e "$required" ]]; then
    printf 'Missing macOS cross-build dependency: %s\nSee docs/development/macos-build-environment.md\n' "$required" >&2
    exit 2
  fi
done
export CC_aarch64_apple_darwin="$cross_root/sdk/bin/arm64-apple-darwin-cc"
export CXX_aarch64_apple_darwin="$cross_root/sdk/bin/arm64-apple-darwin-c++"
export AR_aarch64_apple_darwin="${AR_aarch64_apple_darwin:-/usr/bin/llvm-ar}"
export CC_x86_64_apple_darwin="$cross_root/sdk/bin/x86_64-apple-darwin-cc"
export CXX_x86_64_apple_darwin="$cross_root/sdk/bin/x86_64-apple-darwin-c++"
export AR_x86_64_apple_darwin="${AR_x86_64_apple_darwin:-/usr/bin/llvm-ar}"
export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER="$CC_aarch64_apple_darwin"
export CARGO_TARGET_X86_64_APPLE_DARWIN_LINKER="$CC_x86_64_apple_darwin"
cd "$repo_root"
for target in "${targets[@]}"; do
  arch=x86_64
  if [[ "$target" == aarch64-apple-darwin ]]; then arch=arm64; fi
  export CMAKE_TOOLCHAIN_FILE="$cross_root/sdk/$arch-apple-darwin.toolchain.cmake"
  # Non-interactive WSL console dimensions can trigger Rust 1.95's styled
  # diagnostic renderer panic. Short JSON preserves diagnostics without it.
  cargo "+$toolchain" "$mode" --manifest-path "$manifest" --target "$target" --locked --message-format=json-diagnostic-short "${profile_args[@]}"
done
