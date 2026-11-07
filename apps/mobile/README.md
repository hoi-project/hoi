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

Mobile builds are part of the major-release CI suites. See [development](../../docs/DEVELOPMENT.md)
and [CI](../../docs/CI.md).
