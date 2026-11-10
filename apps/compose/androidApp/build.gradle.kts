import com.android.build.api.variant.ApplicationAndroidComponentsExtension

plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.detekt)
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

android {
    namespace = "app.wonton.android"
    compileSdk = libs.versions.android.compileSdk.get().toInt()

    defaultConfig {
        applicationId = "app.wonton.android"
        minSdk = libs.versions.android.minSdk.get().toInt()
        targetSdk = libs.versions.android.targetSdk.get().toInt()
        versionCode = 1
        versionName = "0.0.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        // 只打包编译了 Rust 核心的 ABI：其他 ABI 的设备直接装不上，而不是装上后因缺 .so 崩溃。
        ndk {
            abiFilters += providers.gradleProperty("wonton.abis").get().split(" ").filter { it.isNotBlank() }
        }
    }

    testOptions {
        unitTests.isIncludeAndroidResources = true
    }
}

dependencies {
    implementation(project(":shared"))
    implementation(libs.androidx.activity.compose)
    // Apache-2.0。应用语言（AppCompatDelegate）：Android 12 及以下由它保存和套用 App 的语言，MainActivity 继承它的 Activity。
    implementation(libs.androidx.appcompat)
    implementation(libs.kotlinx.coroutines.core)
    // Apache-2.0（连带的 Tink 也是）。接 UnifiedPush 分发器：在本机生成 Web Push 密钥对，按 RFC 8291 解开推送。
    implementation(libs.unifiedpush.connector)

    testImplementation(libs.junit)
    testImplementation(libs.robolectric)
    testImplementation(libs.androidx.test.junit)
    // 仪器测试（夜跑用）：启动 App 到首屏的骨架和 Compose 语义树断言。
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.junit)
    androidTestImplementation(libs.androidx.test.rules)
    androidTestImplementation(libs.compose.ui.test.junit4)
    // shared 用 implementation 引 Compose，测试编译 StepCardFrame 要自己拿到这些类型。
    testImplementation(libs.compose.runtime)
    testImplementation(libs.compose.foundation)
    testImplementation(libs.compose.ui.test.junit4)
    debugImplementation(libs.compose.ui.test.manifest)

    // Apache-2.0。设备上的启动测试：rule、runner 和参数化 Activity 场景。
    androidTestImplementation(libs.junit)
    androidTestImplementation(libs.androidx.test.junit)
    androidTestImplementation(libs.androidx.test.runner)
}

/** Robolectric 要读 strings.xml 和各语言的 values-*，资源得进单元测试。 */
tasks.withType<Test>().configureEach {
    // Robolectric 第一次跑会按系统属性里的代理下载 android-all（CI 和本机都靠 https_proxy）。
    listOf("http.proxyHost", "http.proxyPort", "https.proxyHost", "https.proxyPort").forEach { key ->
        System.getProperty(key)?.let { systemProperty(key, it) }
    }
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
