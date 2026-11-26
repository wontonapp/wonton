pluginManagement {
    repositories {
        google()
        mavenCentral()
        gradlePluginPortal()
    }
}

dependencyResolutionManagement {
    repositoriesMode = RepositoriesMode.FAIL_ON_PROJECT_REPOS
    repositories {
        google()
        // Maven Central 对共享 runner 的出口 IP 会回 429，Robolectric 的 SDK jar 又只在配置阶段才拉，整轮作业
        // 还没跑任何任务就红了。Google 官方的 Central 镜像有同一份构件，先走它；镜像还没有的回落 Central。
        maven("https://maven-central.storage-download.googleapis.com/maven2/")
        mavenCentral()
        // rustls-platform-verifier 的 Android 组件只发在它仓库的 maven-archive 分支，不在 Maven Central。
        exclusiveContent {
            forRepository {
                maven("https://raw.githubusercontent.com/rustls/rustls-platform-verifier/maven-archive/android-release-support/maven/")
            }
            filter { includeGroup("org.rustls") }
        }
    }
}

rootProject.name = "wonton"

include(":shared", ":androidApp")
