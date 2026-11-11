import com.android.build.api.variant.KotlinMultiplatformAndroidComponentsExtension

plugins {
    alias(libs.plugins.kotlin.multiplatform)
    alias(libs.plugins.android.kmp.library)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.detekt)
    // Apache-2.0。截图测试只在 JVM 的 host test 里跑，不加载 Rust 的 .so。
    alias(libs.plugins.paparazzi)
}

/** 编译 Rust 客户端核心（cargo-ndk）并生成 Kotlin 绑定，输出交给 AGP 和 Kotlin 编译。 */
abstract class BuildRustCore : Exec() {
    @get:Input
    abstract val rustProfile: Property<String>

    @get:Input
    abstract val minSdk: Property<Int>

    /** 假数据入口（cargo feature ui-fake）：`auto` 按构建类型开关，`1`/`0` 强制。 */
    @get:Input
    abstract val uiFake: Property<String>

    @get:Input
    abstract val abis: ListProperty<String>

    @get:OutputDirectory
    abstract val jniLibsDir: DirectoryProperty

    @get:OutputDirectory
    abstract val kotlinDir: DirectoryProperty

    /** 每个 ABI 一个 .so，产物只有这些；目录本身仍要声明，AGP 拿它当生成的 jniLibs 源目录。 */
    @get:OutputFiles
    abstract val libraries: ConfigurableFileCollection

    init {
        argumentProviders.add {
            listOf(
                jniLibsDir.get().asFile.path,
                kotlinDir.get().asFile.path,
                rustProfile.get(),
                minSdk.get().toString(),
                uiFake.get(),
            ) + abis.get()
        }
    }
}

val repoRoot = rootDir.resolve("../..").normalize()

val buildRustCore = tasks.register<BuildRustCore>("buildRustCore") {
    group = "build"
    description = "用 cargo-ndk 编译 Rust 客户端核心，并生成 UniFFI Kotlin 绑定"
    rustProfile.set(providers.gradleProperty("wonton.rustProfile"))
    // 假数据入口默认按构建类型开关（Debug 带、Release 不带）；夜跑用 -Pwonton.uiFake=1 显式带上。
    uiFake.set(providers.gradleProperty("wonton.uiFake").orElse("auto"))
    minSdk.set(libs.versions.android.minSdk.map { it.toInt() })
    abis.set(providers.gradleProperty("wonton.abis").map { value -> value.split(" ").filter { it.isNotBlank() } })
    kotlinDir.set(layout.buildDirectory.dir("generated/uniffi/kotlin"))
    val script = rootDir.resolve("scripts/build-core.sh")
    executable = script.path
    libraries.from(
        jniLibsDir.zip(abis) { dir: Directory, names: List<String> ->
            names.map { dir.file("$it/libwonton_client.so") }
        },
    )
    // 产物取决于整个工作区：清单、锁文件、工具链和全部源码（vendor/ 的补丁 crate 也算）。漏一项，Gradle 就判输入没变，
    // 拿旧 .so 配新源码。target/ 不列，cargo 自己按它增量重编。
    inputs.file(script)
    inputs.file(repoRoot.resolve("Cargo.toml"))
    inputs.file(repoRoot.resolve("Cargo.lock"))
    inputs.file(repoRoot.resolve("rust-toolchain.toml"))
    // 仓库现在没有 .cargo/，以后加了不列就会漏；fileTree 在目录不存在时为空。
    inputs.files(fileTree(repoRoot.resolve(".cargo")))
    inputs.dir(repoRoot.resolve("crates"))
    inputs.dir(repoRoot.resolve("vendor"))
}

kotlin {
    android {
        namespace = "app.wonton.ui"
        compileSdk = libs.versions.android.compileSdk.get().toInt()
        minSdk = libs.versions.android.minSdk.get().toInt()
        // 噪点贴图（ADR-0026）。
        androidResources { enable = true }
        // 截图测试（Paparazzi）在 JVM 上按布局渲染，要读资源；targetSdk 和 App 一致。
        withHostTest {
            isIncludeAndroidResources = true
            targetSdk { version = release(libs.versions.android.targetSdk.get().toInt()) }
        }
    }

    sourceSets {
        androidMain {
            // UniFFI 的 Kotlin 绑定依赖 JNA，只能编译到 JVM 类目标，所以绑定和界面暂时放在 androidMain。
            kotlin.srcDir(buildRustCore.flatMap { it.kotlinDir })
            dependencies {
                implementation(libs.compose.runtime)
                implementation(libs.compose.foundation)
                implementation(libs.compose.material3)
                // Apache-2.0。@Preview 注解，界面预览和截图测试只看它标注的函数。
                implementation(libs.compose.ui.tooling.preview)
                // Apache-2.0。服务器行使用 Material 的加号和勾选图标。
                implementation("org.jetbrains.compose.material:material-icons-core:1.7.3")
                implementation(libs.kotlinx.coroutines.core)
                // Apache-2.0。桌面快捷方式（ShortcutManagerCompat）和插件图标（IconCompat）。
                implementation(libs.androidx.core)
                // Apache-2.0。应用语言（AppCompatDelegate.setApplicationLocales），设置里的语言选择器。
                implementation(libs.androidx.appcompat)
                // Apache-2.0。WebView 的代理设置（ProxyController），插件网页经核心的本地代理打开。
                implementation(libs.androidx.webkit)
                implementation(libs.androidx.browser)
                // Apache-2.0。把网页媒体状态接到系统媒体会话和默认通知。
                implementation(libs.androidx.media3.session)
                // Apache-2.0。系统的照片选择器（PickVisualMedia）和返回键处理（BackHandler）。
                implementation(libs.androidx.activity.compose)
                // Apache-2.0。扫码配对页的相机取景和逐帧分析（CameraX）。
                implementation(libs.androidx.camera.camera2)
                implementation(libs.androidx.camera.lifecycle)
                implementation(libs.androidx.camera.view)
                // Apache-2.0。在相机的帧里识别二维码（ZXing 的解码库，不含它的扫码界面）。
                implementation(libs.zxing.core)
                // Apache-2.0。Bot 状态头像的静音循环播放（ExoPlayer）和视频头像的转码（Transformer）。
                implementation(libs.media3.exoplayer)
                implementation(libs.media3.transformer)
                implementation(libs.media3.effect)
                implementation("net.java.dev.jna:jna:${libs.versions.jna.get()}@aar")
            }
        }
        // 截图测试（Paparazzi）。预览自己带固定假数据，这里只要断言用的 JUnit。
        getByName("androidHostTest") {
            dependencies {
                implementation(libs.junit)
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
