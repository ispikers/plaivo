plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
    id("com.mikepenz.aboutlibraries.plugin.android")
    id("com.diffplug.spotless")
}

spotless {
    java {
        target("src/**/*.java")
        palantirJavaFormat("2.96.0")
        removeUnusedImports()
    }
    kotlin {
        target("src/**/*.kt")
        ktfmt("0.54").kotlinlangStyle().configure { it.setMaxWidth(120) }
    }
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

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    buildFeatures { compose = true }

    lint { disable += "ChromeOsAbiSupport" }

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
    description = "Builds the plaivo-ffi Rust library for Android ABIs."
    workingDir = workspace
    val out = layout.buildDirectory.dir("rust/jniLibs").get().asFile.absolutePath
    commandLine("cargo", "ndk", "-t", "arm64-v8a", "-t", "armeabi-v7a", "-P", "24", "-o", out, "build", "-p", "plaivo-ffi", "--release", "--config", "profile.release.strip=false", "--config", "profile.release.panic='unwind'")
    doFirst { environment("ANDROID_NDK_HOME", ndkDir.get().asFile.absolutePath) }
}

val generateBindings = tasks.register<Exec>("generateBindings") {
    description = "Generates Kotlin UniFFI bindings from the Rust library."
    dependsOn(buildRust)
    workingDir = workspace
    val lib = File(workspace, "target/aarch64-linux-android/release/libplaivo_ffi.so").absolutePath
    val out = layout.buildDirectory.dir("generated/uniffi").get().asFile.absolutePath
    commandLine("cargo", "run", "-q", "-p", "uniffi-bindgen", "--", "generate", "--library", lib, "--language", "kotlin", "--no-format", "--out-dir", out)
}

tasks.named("preBuild") { dependsOn(generateBindings) }

val licenseConfig = layout.buildDirectory.dir("generated/aboutLibrariesConfig")
val licenseTexts = mapOf("Apache-2.0" to "Apache-2.0.txt", "LGPL-2.1-or-later" to "LGPL-2.1.txt")
val writeLicenseConfig = tasks.register("writeLicenseConfig") {
    description = "Writes AboutLibraries license texts for non-Maven dependencies."
    val sources = licenseTexts.mapValues { File(workspace, "licenses/${it.value}") }
    inputs.files(sources.values)
    outputs.dir(licenseConfig)
    doLast {
        val dir = licenseConfig.get().dir("licenses").asFile.apply { mkdirs() }
        sources.forEach { (id, file) ->
            File(dir, "$id.json").writeText(groovy.json.JsonOutput.toJson(mapOf("name" to id, "hash" to id, "spdxId" to id, "content" to file.readText().replace("\r\n", "\n").trim())))
        }
    }
}
tasks.matching { it.name.startsWith("prepareLibraryDefinitions") }.configureEach { dependsOn(writeLicenseConfig) }

aboutLibraries {
    offlineMode = true
    collect { configPath = licenseConfig.get().asFile }
    export { excludeFields.addAll("description", "developers", "organization", "scm", "funding") }
}

androidComponents.onVariants { variant ->
    val name = variant.name.replaceFirstChar { it.uppercase() }
    val json = layout.buildDirectory.file("generated/aboutLibraries/${variant.name}/res/raw/aboutlibraries.json")
    val graph = configurations.named("${variant.name}RuntimeClasspath").flatMap { it.incoming.resolutionResult.rootComponent }
    val check = tasks.register("check${name}Licenses") {
        description = "Fails if any runtime dependency lacks license text."
        dependsOn("prepareLibraryDefinitions$name")
        inputs.file(json)
        doLast {
            val root = groovy.json.JsonSlurper().parse(json.get().asFile) as Map<*, *>
            val texts = root["licenses"] as Map<*, *>
            val libraries = (root["libraries"] as List<*>).map { it as Map<*, *> }
            val listed = libraries.map { it["uniqueId"] as String }.toSet()
            val problems = libraries.filter { lib ->
                val ids = lib["licenses"] as List<*>? ?: emptyList<Any>()
                ids.isEmpty() || ids.any { ((texts[it] as Map<*, *>?)?.get("content") as String?).isNullOrBlank() }
            }.map { "${it["uniqueId"]} has no license text" }.toMutableList()
            val modules = mutableSetOf<String>()
            fun walk(c: ResolvedComponentResult) {
                c.dependencies.filterIsInstance<ResolvedDependencyResult>().forEach { d ->
                    val id = d.selected.id
                    if ((id is ModuleComponentIdentifier) && modules.add("${id.group}:${id.module}")) walk(d.selected)
                }
            }
            walk(graph.get())
            problems += modules.filter { it !in listed && it.removeSuffix("-android").removeSuffix("-jvm") !in listed }.map { "$it is missing from the license list" }
            if (problems.isNotEmpty()) throw GradleException("Licenses:\n" + problems.joinToString("\n"))
            logger.lifecycle("${libraries.size} Maven libraries, ${modules.size} runtime modules, all licensed")
        }
    }
    tasks.matching { it.name == "assemble$name" }.configureEach { dependsOn(check) }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    implementation("androidx.compose.foundation:foundation")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.tv:tv-material:1.1.0")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.media3:media3-exoplayer:1.11.1")
    implementation("androidx.media3:media3-ui:1.11.1")
    implementation("androidx.media3:media3-exoplayer-hls:1.11.1")
    implementation("io.coil-kt.coil3:coil-compose:3.6.3")
    implementation("io.coil-kt.coil3:coil-network-okhttp:3.6.3")
    implementation("net.java.dev.jna:jna:5.19.1@aar")
}
