# Mobile libraries

This directory contains a Swift package and an Android library over the Rust core,
plus platform key storage and shared design tokens. Standalone mobile apps are not
included.

## Build

From this directory:

```sh
./scripts/build-ios.sh       # macOS, Xcode and the Rust iOS targets
./scripts/build-android.sh   # Android SDK/NDK, cargo-ndk and Android Rust targets
```

The scripts generate UniFFI bindings and native libraries. Generated bindings and
binaries are ignored by Git; shared theme tokens are regenerated with `pnpm tokens`
from the repository root.

## Integrate

Add `ios/HoiCore` as a local Swift package, or include `android/hoi-core` as a Gradle
module in your application. See the wrapper sources for the supported API and events.

The wrappers use Keychain on iOS and Keystore on Android. Application UI calls the
core through `HoiNode`; keys, validation and storage belong to the core. The shared
UI contract is documented by the types in `packages/core-api/src/types.ts`.

The shared Release and Weekly suites skip mobile builds until a platform is ready. To enable
a platform, set `version` in its [Android](android/build.json) or [iOS](ios/build.json)
build configuration to a version such as `"0.1.0"` or `"0.1.0-preview.1"`. The workflow
reads the configuration from the revision being tested. A null, missing, or invalid version
keeps the job skipped. Weekly's Linux-only mode also skips iOS.

These jobs compile the native libraries. See [development](../../docs/DEVELOPMENT.md)
and [CI](../../docs/CI.md).
