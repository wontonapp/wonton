plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.compose.compiler)
    alias(libs.plugins.detekt)
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
}

detekt {
    buildUponDefaultConfig = true
    config.setFrom(rootDir.resolve("config/detekt.yml"))
    source.setFrom("src")
}
