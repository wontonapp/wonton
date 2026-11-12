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
