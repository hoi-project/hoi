// The hoi core for Android apps. Before building, run ../../scripts/build-android.sh: it puts the
// Rust libraries in src/main/jniLibs/<abi>/libhoi_ffi.so and the generated Kotlin bindings in
// src/main/java/uniffi/ (both gitignored).

plugins {
    alias(libs.plugins.android.library)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "sh.hoi.core"
    compileSdk = 35

    defaultConfig {
        minSdk = 26
        consumerProguardFiles("consumer-rules.pro")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    buildFeatures {
        compose = true
    }
}

kotlin {
    jvmToolchain(17)
}

dependencies {
    // UniFFI's Kotlin bindings call Rust through JNA; the aar carries JNA's native part per ABI.
    api(variantOf(libs.jna) { artifactType("aar") })
    // The generated bindings use Android's SystemCleaner (API 33+) behind @RequiresApi.
    implementation(libs.androidx.annotation)
    api(libs.kotlinx.coroutines.android)

    api(platform(libs.androidx.compose.bom))
    api(libs.androidx.compose.ui)
    api(libs.androidx.compose.foundation)
    api(libs.androidx.compose.material3)
}
