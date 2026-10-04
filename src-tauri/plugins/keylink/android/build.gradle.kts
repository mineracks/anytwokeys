plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.anytwokeys.keylink"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

dependencies {
    implementation("androidx.core:core-ktx:1.9.0")
    implementation("androidx.appcompat:appcompat:1.6.0")
    implementation("androidx.biometric:biometric:1.1.0")
    // Yubico's own library, for one thing only: carrying bytes to a YubiKey in the USB port (CCID). What the bytes
    // mean is written once, in Rust.
    implementation("com.yubico.yubikit:android:2.8.2")
    implementation(project(":tauri-android"))
}
