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

        // 只打包编译了 Rust 核心的 ABI：其他 ABI 的设备直接装不上，而不是装上后因缺 .so 崩溃。
        ndk {
            abiFilters += providers.gradleProperty("wonton.abis").get().split(" ").filter { it.isNotBlank() }
        }
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
}

detekt {
    buildUponDefaultConfig = true
    config.setFrom(rootDir.resolve("config/detekt.yml"))
    source.setFrom("src")
}
