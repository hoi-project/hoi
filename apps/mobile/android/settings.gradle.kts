// hoi for Android: the `hoi-core` library module (Rust core via UniFFI + Kotlin wrapper + theme).
// Add it to your own app with `includeBuild` or by copying the module; see ../README.md.

pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode.set(RepositoriesMode.FAIL_ON_PROJECT_REPOS)
    repositories {
        google()
        mavenCentral()
    }
}

rootProject.name = "hoi-mobile"
include(":hoi-core")
