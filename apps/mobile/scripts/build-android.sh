#!/usr/bin/env bash
# Builds the Rust core for Android and the Kotlin bindings for android/hoi-core:
#
#   android/hoi-core/src/main/jniLibs/<abi>/libhoi_ffi.so    arm64-v8a, armeabi-v7a, x86_64
#   android/hoi-core/src/main/java/uniffi/                   generated Kotlin (uniffi.hoi_ffi, uniffi.hoi_core)
#
# Both are gitignored. Needs the Android NDK r27+ (ANDROID_NDK_HOME, or ANDROID_HOME with an
# ndk/ folder) and cargo-ndk. Works on macOS and Linux:
#
#   apps/mobile/scripts/build-android.sh            release build (what you ship)
#   apps/mobile/scripts/build-android.sh --debug    quicker builds while you work on the app
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
mobile="$(cd "$here/.." && pwd)"
repo="$(cd "$mobile/../.." && pwd)"
core="$repo/core"
module="$mobile/android/hoi-core"
target_dir="${CARGO_TARGET_DIR:-$core/target}"

release=(--release)
for arg in "$@"; do
  case "$arg" in
    --debug) release=() ;;
    --release) release=(--release) ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "build-android: unknown option $arg (try --help)" >&2; exit 2 ;;
  esac
done

die() { echo "build-android: $*" >&2; exit 1; }

command -v cargo >/dev/null || die "cargo not found. Install Rust with rustup: https://rustup.rs"
command -v rustup >/dev/null || die "rustup not found; the Android targets are added with it. https://rustup.rs"
command -v cargo-ndk >/dev/null || die "cargo-ndk not found. Run: cargo install cargo-ndk"
if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
  sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
  [[ -n "$sdk" && -d "$sdk/ndk" ]] || die "no Android NDK found. Install NDK r27+ (Android Studio → SDK Manager → SDK Tools → NDK) and set ANDROID_HOME, or set ANDROID_NDK_HOME."
fi

targets=(aarch64-linux-android armv7-linux-androideabi x86_64-linux-android)
cd "$core" # rust-toolchain.toml here picks the toolchain the targets belong to
installed="$(rustup target list --installed)"
missing=()
for t in "${targets[@]}"; do
  grep -qx "$t" <<<"$installed" || missing+=("$t")
done
[[ ${#missing[@]} -eq 0 ]] || die "missing Rust targets. Run, from core/: rustup target add ${missing[*]}"

# Google Play wants 16 KB page alignment for native code (Android 15+). NDK r28 does it by
# default; this keeps r27 right too. Set per target so host builds stay untouched.
for t in AARCH64_LINUX_ANDROID ARMV7_LINUX_ANDROIDEABI X86_64_LINUX_ANDROID; do
  var="CARGO_TARGET_${t}_RUSTFLAGS"
  export "$var=${!var:-} -C link-arg=-Wl,-z,max-page-size=16384"
done

jni="$module/src/main/jniLibs"
echo "==> Rust core for arm64-v8a, armeabi-v7a, x86_64"
rm -rf "$jni"
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 --platform 26 -o "$jni" build -p hoi-ffi --lib ${release[@]+"${release[@]}"}

echo "==> Kotlin bindings"
cargo build -p hoi-ffi --features cli --bin uniffi-bindgen
out="$module/src/main/java"
rm -rf "$out/uniffi"
mkdir -p "$out"
"$target_dir/debug/uniffi-bindgen" generate \
  --library "$jni/arm64-v8a/libhoi_ffi.so" \
  --language kotlin \
  --config "$core/crates/hoi-ffi/bindgen.toml" \
  --out-dir "$out"

echo "==> done"
find "$jni" -name '*.so' | sed 's/^/    /'
find "$out/uniffi" -name '*.kt' | sed 's/^/    /'
echo "Open $mobile/android in Android Studio, or include :hoi-core in your app (see apps/mobile/README.md)."
