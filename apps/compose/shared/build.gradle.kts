import com.android.build.api.variant.KotlinMultiplatformAndroidComponentsExtension

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.android.kmp.library)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.detekt)
}

/** 编译 Rust 客户端核心（cargo-ndk）并生成 Kotlin 绑定，输出交给 AGP 和 Kotlin 编译。 */
abstract class BuildRustCore : Exec() {
    @get:Input
    abstract val rustProfile: Property<String>

    @get:Input
    abstract val minSdk: Property<Int>

    @get:Input
    abstract val abis: ListProperty<String>

    @get:OutputDirectory
    abstract val jniLibsDir: DirectoryProperty

    @get:OutputDirectory
    abstract val kotlinDir: DirectoryProperty

    init {
        argumentProviders.add {
            listOf(
                jniLibsDir.get().asFile.path,
                kotlinDir.get().asFile.path,
                rustProfile.get(),
                minSdk.get().toString(),
            ) + abis.get()
        }
    }
}

val repoRoot = rootDir.resolve("../..").normalize()

val buildRustCore = tasks.register<BuildRustCore>("buildRustCore") {
    group = "build"
    description = "用 cargo-ndk 编译 Rust 客户端核心，并生成 UniFFI Kotlin 绑定"
    rustProfile.set(providers.gradleProperty("wonton.rustProfile"))
    minSdk.set(libs.versions.android.minSdk.map { it.toInt() })
    abis.set(providers.gradleProperty("wonton.abis").map { value -> value.split(" ").filter { it.isNotBlank() } })
    kotlinDir.set(layout.buildDirectory.dir("generated/uniffi/kotlin"))
    val script = rootDir.resolve("scripts/build-core.sh")
    executable = script.path
    inputs.file(script)
    inputs.dir(repoRoot.resolve("crates/wonton-client"))
    inputs.dir(repoRoot.resolve("crates/wonton-matrix"))
    inputs.dir(repoRoot.resolve("crates/wonton-protocol"))
    inputs.dir(repoRoot.resolve("crates/wonton-tunnel"))
    inputs.file(repoRoot.resolve("Cargo.lock"))
}

kotlin {
    android {
        namespace = "app.wonton.ui"
        compileSdk = libs.versions.android.compileSdk.get().toInt()
        minSdk = libs.versions.android.minSdk.get().toInt()
        // 噪点贴图（ADR-0026）。
        androidResources { enable = true }
    }

    sourceSets {
        androidMain {
            // UniFFI 的 Kotlin 绑定依赖 JNA，只能编译到 JVM 类目标，所以绑定和界面暂时放在 androidMain。
            kotlin.srcDir(buildRustCore.flatMap { it.kotlinDir })
            dependencies {
                implementation(libs.compose.runtime)
                implementation(libs.compose.foundation)
                implementation(libs.compose.material3)
                implementation(libs.kotlinx.coroutines.core)
                // Apache-2.0。桌面快捷方式（ShortcutManagerCompat）和插件图标（IconCompat）。
                implementation(libs.androidx.core)
                // Apache-2.0。WebView 的代理设置（ProxyController），插件网页经核心的本地代理打开。
                implementation(libs.androidx.webkit)
                // Apache-2.0。系统的照片选择器（PickVisualMedia）和返回键处理（BackHandler）。
                implementation(libs.androidx.activity.compose)
                // Apache-2.0。扫码配对页的相机取景和逐帧分析（CameraX）。
                implementation(libs.androidx.camera.camera2)
                implementation(libs.androidx.camera.lifecycle)
                implementation(libs.androidx.camera.view)
                // Apache-2.0。在相机的帧里识别二维码（ZXing 的解码库，不含它的扫码界面）。
                implementation(libs.zxing.core)
                implementation("net.java.dev.jna:jna:${libs.versions.jna.get()}@aar")
            }
        }
    }
}

extensions.configure<KotlinMultiplatformAndroidComponentsExtension> {
    onVariants { variant ->
        variant.sources.jniLibs?.addGeneratedSourceDirectory(buildRustCore, BuildRustCore::jniLibsDir)
    }
}

detekt {
    buildUponDefaultConfig = true
    config.setFrom(rootDir.resolve("config/detekt.yml"))
    // scripts/generate-design.py 的输出不手改，不查。
    source.setFrom(fileTree("src") { exclude("**/ui/icons/**", "**/ui/design/generated/**") })
}
