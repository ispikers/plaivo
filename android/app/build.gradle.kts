plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "xyz.spikers.plaivo"
    compileSdk = 37
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "xyz.spikers.plaivo"
        minSdk = 24
        targetSdk = 37
        versionCode = 1
        versionName = "0.1.0"
        ndk { abiFilters += listOf("arm64-v8a", "armeabi-v7a") }
    }

    buildFeatures { compose = true }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

androidComponents.onVariants { variant ->
    variant.sources.java?.addStaticSourceDirectory("build/generated/uniffi")
    variant.sources.jniLibs?.addStaticSourceDirectory("build/rust/jniLibs")
}

val workspace: File = rootProject.projectDir.parentFile
val ndkDir = androidComponents.sdkComponents.ndkDirectory

val buildRust = tasks.register<Exec>("buildRust") {
    workingDir = workspace
    val out = layout.buildDirectory.dir("rust/jniLibs").get().asFile.absolutePath
    commandLine("cargo", "ndk", "-t", "arm64-v8a", "-t", "armeabi-v7a", "-P", "24", "-o", out, "build", "-p", "plaivo-ffi", "--release", "--config", "profile.release.strip=false")
    doFirst { environment("ANDROID_NDK_HOME", ndkDir.get().asFile.absolutePath) }
}

val generateBindings = tasks.register<Exec>("generateBindings") {
    dependsOn(buildRust)
    workingDir = workspace
    val lib = File(workspace, "target/aarch64-linux-android/release/libplaivo_ffi.so").absolutePath
    val out = layout.buildDirectory.dir("generated/uniffi").get().asFile.absolutePath
    commandLine("cargo", "run", "-q", "-p", "uniffi-bindgen", "--", "generate", "--library", lib, "--language", "kotlin", "--no-format", "--out-dir", out)
}

tasks.named("preBuild") { dependsOn(generateBindings) }

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.tv:tv-material:1.1.0")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.media3:media3-exoplayer:1.11.1")
    implementation("androidx.media3:media3-ui:1.11.1")
    implementation("androidx.media3:media3-exoplayer-hls:1.11.1")
    implementation("io.coil-kt.coil3:coil-compose:3.6.3")
    implementation("io.coil-kt.coil3:coil-network-okhttp:3.6.3")
    implementation("net.java.dev.jna:jna:5.19.1@aar")
}
