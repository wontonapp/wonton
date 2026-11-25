import com.android.build.api.variant.ApplicationAndroidComponentsExtension

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.detekt)
    alias(libs.plugins.aboutlibraries)
}

/**
 * 生成系统“应用语言”页要的 res/xml/locales_config.xml：列出 App 和 shared 库里每个带 strings.xml 的 values-* 目录。
 * AGP 的 generateLocaleConfig 只看 App 模块自己的资源，shared 库里的语言会漏掉，所以自己生成。
 */
abstract class GenerateLocalesConfig : DefaultTask() {
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val stringsFiles: ConfigurableFileCollection

    /** values/（不带限定符的目录）对应的语言。 */
    @get:Input
    abstract val baseLocale: Property<String>

    @get:OutputDirectory
    abstract val outputDir: DirectoryProperty

    @TaskAction
    fun generate() {
        val base = baseLocale.get()
        val tags = stringsFiles.files
            .map { it.parentFile.name }
            .distinct()
            .map { dir -> if (dir == "values") base else valuesDirTag(dir) }
            .distinct()
            .sortedWith(compareBy({ it != base }, { it }))
        val xml = buildString {
            appendLine("<?xml version=\"1.0\" encoding=\"utf-8\"?>")
            appendLine("<!-- 由 androidApp/build.gradle.kts 的 generateLocalesConfig 任务生成，不要手改。 -->")
            appendLine("<locale-config xmlns:android=\"http://schemas.android.com/apk/res/android\" android:defaultLocale=\"$base\">")
            tags.forEach { appendLine("    <locale android:name=\"$it\"/>") }
            appendLine("</locale-config>")
        }
        val file = outputDir.get().file("xml/locales_config.xml").asFile
        file.parentFile.mkdirs()
        file.writeText(xml)
    }

    /** values-ja → ja，values-zh-rTW → zh-TW，values-b+zh+Hans → zh-Hans；其他限定符直接失败，不猜。 */
    private fun valuesDirTag(dir: String): String {
        val qualifier = dir.removePrefix("values-")
        return when {
            qualifier.startsWith("b+") -> qualifier.removePrefix("b+").replace('+', '-')
            Regex("[a-z]{2,3}(-r[A-Z]{2})?").matches(qualifier) -> qualifier.replace("-r", "-")
            else -> throw GradleException(
                "$dir 的限定符不支持：只认 values-<语言>、values-<语言>-r<地区> 和 values-b+<语言>[+<文字>]",
            )
        }
    }
}

val generateLocalesConfig = tasks.register<GenerateLocalesConfig>("generateLocalesConfig") {
    group = "build"
    description = "生成系统“应用语言”页用的 locales_config.xml（App 和 shared 库里的 values-* 语言）"
    stringsFiles.from(
        fileTree("src/main/res") { include("values*/strings.xml") },
        project(":shared").fileTree("src/androidMain/res") { include("values*/strings.xml") },
    )
    baseLocale.set("en")
    outputDir.set(layout.buildDirectory.dir("generated/localesConfig/res"))
}

extensions.configure<ApplicationAndroidComponentsExtension> {
    onVariants { variant ->
        variant.sources.res?.addGeneratedSourceDirectory(generateLocalesConfig, GenerateLocalesConfig::outputDir)
    }
}

/**
 * rustls-platform-verifier 的 Kotlin 组件版本必须等于 Cargo.lock 里 rustls-platform-verifier-android 的版本，
 * 不一致时 Rust 那边找不到要调的类或方法。做法照它 README 的 Android 一节。
 */
abstract class RustlsPlatformVerifierVersion : ValueSource<String, RustlsPlatformVerifierVersion.Params> {
    interface Params : ValueSourceParameters {
        val lockFile: RegularFileProperty
    }

    override fun obtain(): String {
        val lines = parameters.lockFile.get().asFile.readLines()
        val nameIdx = lines.indexOfFirst { it.trim() == "name = \"$CRATE\"" }
        return lines.drop(nameIdx + 1)
            .takeIf { nameIdx >= 0 }
            ?.firstOrNull { it.trimStart().startsWith("version = ") }
            ?.substringAfter('"')
            ?.substringBefore('"')
            ?.takeIf { it.isNotEmpty() }
            ?: throw GradleException("$CRATE not found in ${parameters.lockFile.get().asFile}")
    }

    private companion object {
        const val CRATE = "rustls-platform-verifier-android"
    }
}

val rustlsPlatformVerifierVersion = providers.of(RustlsPlatformVerifierVersion::class.java) {
    parameters.lockFile.set(rootProject.layout.projectDirectory.file("../../Cargo.lock"))
}

configurations.configureEach {
    resolutionStrategy.eachDependency {
        if (requested.group == "org.rustls" && requested.name == "rustls-platform-verifier") {
            useVersion(rustlsPlatformVerifierVersion.get())
            because("must match rustls-platform-verifier-android in Cargo.lock")
        }
    }
}

// versionName/versionCode 由仓库根的 scripts/app-version.sh 一处计算（iOS 共用同一规则）：
// 环境变量 WONTON_VERSION（发版脚本显式传，-PwontonVersion 透传给它）→ 最近的 vX.Y.Z tag；
// 都拿不到时 Debug 用 0.0.0/0（CI 浅克隆没有 tag），Release 直接失败，不悄悄落到 0.0.0。
// providers.exec 惰性求值，值留到该变体真正合并 manifest 的任务里才取：编 Debug 不会因为 Release
// 变体拿不到版本号而在配置期失败；spec 里只捕获可序列化的值，配置缓存才存得下。
// 构建可能搬到没有本仓库目录结构的机器上跑（android-remote 的远程副本），路径必须按本次构建的
// rootDir 现场解析，不能写死任何机器路径。
val versionScript = rootDir.parentFile.parentFile.resolve("scripts/app-version.sh")
require(versionScript.isFile) { "找不到版本号脚本 ${versionScript.path}（rootDir=${rootDir.path}）" }

/**
 * 生成关于页“开源许可”用的 assets/licenses.json：Rust crate 经 cargo-about 从 Cargo.lock 取，
 * Android 依赖读 AboutLibraries 刚导出的 aboutlibraries.json，许可全文一并带上。
 */
abstract class GenerateLicenses : DefaultTask() {
    @get:InputFile
    abstract val script: RegularFileProperty

    @get:InputFile
    abstract val androidExport: RegularFileProperty

    @get:InputFile
    abstract val approved: RegularFileProperty

    /** 脚本按自己的路径定位仓库根，这里只当工作目录；不拿它当输入。 */
    @get:Internal
    abstract val repo: DirectoryProperty

    /**
     * 只把脚本要读的文件列成输入：整个仓库还含 target/ 和 build/，cargo 并发写它们会让 Gradle
     * 判“输入不可读”（删掉的 .o），build/ 里的输出还会和本任务形成隐式依赖。
     */
    @get:InputFiles
    @get:PathSensitive(PathSensitivity.RELATIVE)
    abstract val sources: ConfigurableFileCollection

    @get:OutputDirectory
    abstract val outDir: DirectoryProperty

    @get:Inject
    abstract val exec: ExecOperations

    @TaskAction
    fun generate() {
        outDir.get().asFile.mkdirs()
        exec.exec {
            commandLine(
                "python3",
                script.get().asFile.absolutePath,
                "--platform",
                "android",
                "--android-export",
                androidExport.get().asFile.absolutePath,
                "--out",
                outDir.get().asFile.absolutePath,
            )
            workingDir = repo.get().asFile
        }
    }
}

/** 仓库根：androidApp 在 apps/compose/androidApp。 */
val repoRoot = rootDir.parentFile.parentFile

/** licenses/APPROVED 里的许可 id（每行第一段），和 Rust 侧 cargo-about 的 accepted 是同一张表。 */
val approvedLicenses = providers.fileContents(layout.projectDirectory.file("../../../licenses/APPROVED")).asText.map { text ->
    text.lineSequence().filter { "|" in it && !it.startsWith("格式") }
        .map { it.substringBefore('|').trim() }.filter { it.isNotEmpty() }.toList()
}

aboutLibraries {
    // 许可全文由插件取（SPDX 文本，取过就进 Gradle 缓存）；离线模式下它不回填文本，所以不开。
    offlineMode = false
    license {
        // 依赖里出现 APPROVED 之外（含 GPL、AGPL）的许可就让构建失败，等人看过再补。
        strictMode = com.mikepenz.aboutlibraries.plugin.StrictMode.FAIL
        allowedLicenses.addAll(approvedLicenses.get())
        additionalLicenses.addAll(approvedLicenses.get())
    }
}

extensions.configure<ApplicationAndroidComponentsExtension> {
    onVariants { variant ->
        val generateLicenses = tasks.register<GenerateLicenses>("generateLicenses${variant.name.replaceFirstChar { it.uppercase() }}") {
            script.set(repoRoot.resolve("scripts/generate-licenses.py"))
            approved.set(repoRoot.resolve("licenses/APPROVED"))
            repo.set(repoRoot)
            sources.from(
                project.fileTree(repoRoot) { include("Cargo.lock", "crates/*/Cargo.toml", "licenses/**") },
            )
            androidExport.set(
                layout.buildDirectory.file("generated/aboutLibraries/${variant.name}/res/raw/aboutlibraries.json"),
            )
            // 同一个文件来自 AboutLibraries 的导出任务；它的输出路径就是上面这个约定位置。
            dependsOn("prepareLibraryDefinitions${variant.name.replaceFirstChar { it.uppercase() }}")
        }
        variant.sources.assets?.addGeneratedSourceDirectory(generateLicenses, GenerateLicenses::outDir)

        val buildType = variant.buildType
        val wontonVersionProperty = providers.gradleProperty("wontonVersion").orNull
        val appVersion = providers.exec {
            // 失败不抛：providers.exec 在非零退出时既不回显 stderr、也不交出已捕获的 stdout，Gradle 只报
            // exit code，“发版要传 WONTON_VERSION…”的人话就看不见了。stderr 合进 stdout、忽略退出码，
            // 让下面的解析在拿不到 VERSION/BUILD 两行时把脚本原话带进异常。
            isIgnoreExitValue = true
            commandLine("bash", "-c", "\"\$1\" \"\$2\" 2>&1", "app-version", versionScript.absolutePath, buildType)
            if (wontonVersionProperty != null) {
                environment("WONTON_VERSION", wontonVersionProperty)
            }
        }.standardOutput.asText.map { output ->
            val values = output.lines().filter { it.contains('=') }.associate {
                val (key, value) = it.split("=", limit = 2)
                key.trim() to value.trim()
            }
            val version = values["VERSION"]
            val build = values["BUILD"]?.toIntOrNull()
            if (version == null || build == null) {
                throw GradleException("scripts/app-version.sh 没输出 VERSION=X.Y.Z 和 BUILD=<数字> 两行：\n$output")
            }
            version to build
        }
        variant.outputs.forEach { output ->
            output.versionName.set(appVersion.map { it.first })
            output.versionCode.set(appVersion.map { it.second })
        }
    }
}

android {
    namespace = "app.wonton.android"
    compileSdk = libs.versions.android.compileSdk.get().toInt()

    defaultConfig {
        applicationId = "app.wonton.android"
        minSdk = libs.versions.android.minSdk.get().toInt()
        targetSdk = libs.versions.android.targetSdk.get().toInt()
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
        testInstrumentationRunnerArguments["clearPackageData"] = "true"
        // 只打包编译了 Rust 核心的 ABI：其他 ABI 的设备直接装不上，而不是装上后因缺 .so 崩溃。
        ndk {
            abiFilters += providers.gradleProperty("wonton.abis").get().split(" ").filter { it.isNotBlank() }
        }
        // FCM（ADR-0003 的“渠道”）：这些值来自 Firebase 项目和我们的推送中继，属于构建参数。没配时
        // `FcmPush.configured` 为假，App 用 UnifiedPush 分发器，不假装推送已经开通。
        buildConfigField("String", "FCM_APP_ID", "\"${providers.gradleProperty("wonton.fcmAppId").getOrElse("")}\"")
        buildConfigField("String", "FCM_PROJECT_ID", "\"${providers.gradleProperty("wonton.fcmProjectId").getOrElse("")}\"")
        buildConfigField("String", "FCM_API_KEY", "\"${providers.gradleProperty("wonton.fcmApiKey").getOrElse("")}\"")
        buildConfigField("String", "FCM_SENDER_ID", "\"${providers.gradleProperty("wonton.fcmSenderId").getOrElse("")}\"")
        buildConfigField("String", "PUSH_RELAY", "\"${providers.gradleProperty("wonton.pushRelay").getOrElse("")}\"")
    }

    buildFeatures {
        buildConfig = true
    }

    buildTypes {
        release {
            // 直装包要小：R8 删掉没人用的类和成员，顺带缩资源。反射入口的 keep 规则在 proguard-rules.pro。
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"), "proguard-rules.pro")
        }
    }

    packaging {
        resources {
            // 只有 META-INF 里的许可证和版本文件，运行时用不到。
            excludes += setOf(
                "META-INF/LICENSE*",
                "META-INF/NOTICE*",
                "META-INF/AL2.0",
                "META-INF/LGPL2.1",
                "META-INF/*.version",
                "META-INF/*.kotlin_module",
            )
        }
        // 压缩 .so：默认按 4 KB 对齐、不压缩存进 APK，几个大 .so 就让包大一截。打开后安装时解出来，
        // 下载量按压缩后算；代价是设备上要多占解压出的 .so（arm64 约 110 MB），不打开则直接映射 APK 里的副本。
        jniLibs {
            useLegacyPackaging = true
        }
    }

    testOptions {
        execution = "ANDROIDX_TEST_ORCHESTRATOR"
        unitTests.isIncludeAndroidResources = true
    }
}

// Robolectric 默认在测试进程里自己连 Maven 拉 android-all-instrumented，绕开 Gradle 的依赖缓存，网络一抖测试就失败。
// 改由 Gradle 解析（进 Gradle 缓存，CI 由 setup-gradle 缓存），同步成 Robolectric 认的平铺目录（<artifactId>-<version>.jar），
// 测试里开 robolectric.offline 只读这个目录。缺哪个 SDK 会在测试里报 Unable to locate dependency，不会退回联网。
// 每个 SDK 一个 configuration：同一模块的几个版本放在一起，Gradle 冲突解析只留最高的那个。
fun robolectricSdk(name: String): Configuration = configurations.create(name) {
    isCanBeConsumed = false
    isTransitive = false
}
val robolectricSdk32 = robolectricSdk("robolectricSdk32")
val robolectricSdk36 = robolectricSdk("robolectricSdk36")

val robolectricSdkDir = layout.buildDirectory.dir("robolectric-sdk")

val prepareRobolectricSdks = tasks.register<Sync>("prepareRobolectricSdks") {
    from(robolectricSdk32, robolectricSdk36)
    into(robolectricSdkDir)
}

dependencies {
    implementation(project(":shared"))
    implementation(libs.androidx.activity.compose)
    // Apache-2.0。应用语言（AppCompatDelegate）：Android 12 及以下由它保存和套用 App 的语言，MainActivity 继承它的 Activity。
    implementation(libs.androidx.appcompat)
    implementation(libs.kotlinx.coroutines.core)
    implementation(libs.androidx.work.runtime)
    // Apache-2.0（连带的 Tink 也是）。接 UnifiedPush 分发器：在本机生成 Web Push 密钥对，按 RFC 8291 解开推送。
    implementation(libs.unifiedpush.connector)
    implementation(libs.firebase.messaging)
    implementation(libs.play.services.base)
    implementation(libs.rustls.platform.verifier)

    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    testImplementation(libs.androidx.test.junit)
    robolectricSdk32(libs.robolectric.android.all.sdk32)
    robolectricSdk36(libs.robolectric.android.all.sdk36)
    // 仪器测试（夜跑用）：启动 App 到首屏的骨架和 Compose 语义树断言。
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.junit)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.compose.foundation)
    androidTestImplementation(libs.compose.ui.test.junit4)
    // shared 用 implementation 引 Compose，测试编译 StepCardFrame 要自己拿到这些类型。
    testImplementation(libs.compose.runtime)
    testImplementation(libs.compose.foundation)
    testImplementation(libs.compose.ui.test.junit4)
    debugImplementation(libs.compose.ui.test.manifest)
    // Debug 变体的走查页（QualityPreviewActivity）直接摆 Compose 行：只在 Debug 带上。
    debugImplementation(libs.compose.foundation)

    // Apache-2.0。设备上跑 androidTest 的 runner（testInstrumentationRunner 指它）；
    // AndroidJUnitRunner 在这个包里，ext:junit 不带。
    androidTestImplementation(libs.androidx.test.runner)
    // Apache-2.0。每条设备用例独立启动测试进程，并在用例间清理 App 数据。
    androidTestUtil(libs.androidx.test.orchestrator)
}

/** Robolectric 要读 strings.xml 和各语言的 values-*，资源得进单元测试。 */
tasks.withType<Test>().configureEach {
    // 换 SDK jar 要让测试重跑；Robolectric 按文件名找，绝对路径不进缓存键（所以目录走参数提供者，不写进 systemProperty）。
    inputs.files(prepareRobolectricSdks)
        .withPropertyName("robolectricSdks")
        .withPathSensitivity(PathSensitivity.NAME_ONLY)
    systemProperty("robolectric.offline", "true")
    val sdkDir = robolectricSdkDir
    jvmArgumentProviders.add(
        CommandLineArgumentProvider {
            listOf("-Drobolectric.dependency.dir=${sdkDir.get().asFile.absolutePath}")
        },
    )
    // JDK 21 默认不导出 Robolectric 在 API 36 上要用的内部类（jdk.internal.access、FileDescriptor）。
    jvmArgs(
        "--add-exports=java.base/jdk.internal.access=ALL-UNNAMED",
        "--add-opens=java.base/java.io=ALL-UNNAMED",
    )
}

detekt {
    buildUponDefaultConfig = true
    config.setFrom(rootDir.resolve("config/detekt.yml"))
    source.setFrom("src")
}
