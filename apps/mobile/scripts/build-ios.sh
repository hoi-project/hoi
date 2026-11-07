#!/usr/bin/env bash
# Builds the Rust core for iOS and the Swift bindings for ios/HoiCore:
#
#   ios/HoiCore/Frameworks/HoiCoreFFI.xcframework   static libraries + C headers (module HoiCoreFFI)
#   ios/HoiCore/Sources/HoiCoreFFI/*.swift          generated Swift bindings (compiled into HoiCore)
#
# Both are gitignored. Run on macOS with Xcode 16+:
#
#   apps/mobile/scripts/build-ios.sh            release build (what you ship; slow: LTO)
#   apps/mobile/scripts/build-ios.sh --debug    quicker builds while you work on the app
#   apps/mobile/scripts/build-ios.sh --macos    also add a macOS slice (SwiftUI previews on Mac, `swift test`)
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mobile="$(cd "$here/.." && pwd)"
repo="$(cd "$mobile/../.." && pwd)"
core="$repo/core"
pkg="$mobile/ios/HoiCore"
target_dir="${CARGO_TARGET_DIR:-$core/target}"

profile=release
macos=0
for arg in "$@"; do
  case "$arg" in
    --debug) profile=dev ;;
    --release) profile=release ;;
    --macos) macos=1 ;;
    -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
    *) echo "build-ios: unknown option $arg (try --help)" >&2; exit 2 ;;
  esac
done
out_profile=$([[ $profile == dev ]] && echo debug || echo release)

die() { echo "build-ios: $*" >&2; exit 1; }

[[ "$(uname -s)" == Darwin ]] || die "this needs macOS with Xcode (xcodebuild, lipo). On Linux, build-android.sh works."
command -v xcodebuild >/dev/null || die "xcodebuild not found. Install Xcode 16+ and run: sudo xcode-select -s /Applications/Xcode.app"
command -v lipo >/dev/null || die "lipo not found. Install the Xcode command line tools: xcode-select --install"
command -v cargo >/dev/null || die "cargo not found. Install Rust with rustup: https://rustup.rs"
command -v rustup >/dev/null || die "rustup not found; the iOS targets are added with it. https://rustup.rs"
xcode_major="$(xcodebuild -version | awk 'NR==1 {print $2}' | cut -d. -f1)"
[[ "${xcode_major:-0}" -ge 16 ]] || die "Xcode 16 or newer needed (found $(xcodebuild -version | head -1))."

ios_targets=(aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios)
mac_targets=(aarch64-apple-darwin x86_64-apple-darwin)
targets=("${ios_targets[@]}")
[[ $macos == 1 ]] && targets+=("${mac_targets[@]}")

cd "$core" # rust-toolchain.toml here picks the toolchain the targets belong to
installed="$(rustup target list --installed)"
missing=()
for t in "${targets[@]}"; do
  grep -qx "$t" <<<"$installed" || missing+=("$t")
done
[[ ${#missing[@]} -eq 0 ]] || die "missing Rust targets. Run, from core/: rustup target add ${missing[*]}"

export IPHONEOS_DEPLOYMENT_TARGET=16.0
export MACOSX_DEPLOYMENT_TARGET=13.0

echo "==> Rust core ($profile) for ${targets[*]}"
for t in "${targets[@]}"; do
  cargo rustc -p hoi-ffi --lib --crate-type staticlib --profile "$profile" --target "$t"
done
lib() { echo "$target_dir/$1/$out_profile/libhoi_ffi.a"; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/sim" "$work/mac" "$work/swift" "$work/headers/HoiCoreFFI"

echo "==> one simulator library for arm64 and x86_64"
lipo -create "$(lib aarch64-apple-ios-sim)" "$(lib x86_64-apple-ios)" -output "$work/sim/libhoi_ffi.a"
if [[ $macos == 1 ]]; then
  lipo -create "$(lib aarch64-apple-darwin)" "$(lib x86_64-apple-darwin)" -output "$work/mac/libhoi_ffi.a"
fi

echo "==> Swift bindings"
cargo build -p hoi-ffi --features cli --bin uniffi-bindgen
"$target_dir/debug/uniffi-bindgen" generate \
  --library "$(lib aarch64-apple-ios)" \
  --language swift \
  --config "$core/crates/hoi-ffi/bindgen.toml" \
  --out-dir "$work/swift"

# Both crates' headers in one Clang module, in a folder of its own so the module map never
# collides with other xcframeworks' (Xcode copies every Headers/ into one include directory).
cp "$work/swift/"*.h "$work/headers/HoiCoreFFI/"
{
  echo "module HoiCoreFFI {"
  for h in "$work/headers/HoiCoreFFI/"*.h; do echo "    header \"$(basename "$h")\""; done
  echo "    export *"
  echo "    use \"Darwin\""
  echo "    use \"_Builtin_stdbool\""
  echo "    use \"_Builtin_stdint\""
  echo "}"
} >"$work/headers/HoiCoreFFI/module.modulemap"

echo "==> HoiCoreFFI.xcframework"
xcf="$pkg/Frameworks/HoiCoreFFI.xcframework"
rm -rf "$xcf"
mkdir -p "$pkg/Frameworks"
args=(-library "$(lib aarch64-apple-ios)" -headers "$work/headers"
      -library "$work/sim/libhoi_ffi.a" -headers "$work/headers")
[[ $macos == 1 ]] && args+=(-library "$work/mac/libhoi_ffi.a" -headers "$work/headers")
xcodebuild -create-xcframework "${args[@]}" -output "$xcf" >/dev/null

gen="$pkg/Sources/HoiCoreFFI"
rm -rf "$gen"
mkdir -p "$gen"
cp "$work/swift/"*.swift "$gen/"

echo "==> done"
echo "    $xcf"
echo "    $gen/$(cd "$gen" && ls *.swift | tr '\n' ' ')"
echo "Add $pkg to your Xcode project as a local package and link the HoiCore (and HoiTheme) products."
