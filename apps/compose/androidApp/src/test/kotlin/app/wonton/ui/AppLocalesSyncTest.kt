package app.wonton.ui

import android.app.Application
import android.app.LocaleManager
import android.content.res.Configuration
import android.os.LocaleList
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.core.app.AppLocalesStorageHelper
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * 语言的两条链路：系统“应用语言”要以 AppCompat 的存储同步（AppLanguage.kt），否则 Activity 首次
 * attach 的同步见存储为空就把空列表写回系统，抹掉用户选的语言；给核心的语言第一项取自 App 语言
 * （language_tag 资源），其后是系统的语言偏好（WontonApp 的 PluginsEffects 发 SetLanguages）。
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
// 不起 WontonApplication：它的静态初始化要加载 Android 的原生库，JVM 上装不进。
@Config(sdk = [36], qualifiers = "b+zh+Hans", application = Application::class)
class AppLocalesSyncTest {
    @get:Rule
    val compose = createComposeRule()

    private val context = ApplicationProvider.getApplicationContext<Application>()

    @Test
    fun systemLocaleIsSeededIntoStorage() {
        appLocales(LocaleList.forLanguageTags("zh-Hans-CN"))
        syncAppLocalesFromSystem(context)
        assertEquals("zh-Hans-CN", AppLocalesStorageHelper.readLocales(context))
    }

    @Test
    fun emptySystemLocaleLeavesStorageAlone() {
        // 用户在系统设置里清掉了 App 语言：存储要跟着为空，不能拿旧值顶回去。
        AppLocalesStorageHelper.persistLocales(context, "ja")
        appLocales(LocaleList.getEmptyLocaleList())
        syncAppLocalesFromSystem(context)
        assertEquals("ja", AppLocalesStorageHelper.readLocales(context))
    }

    @Test
    fun systemLocaleWinsOverStaleStorage() {
        AppLocalesStorageHelper.persistLocales(context, "ja")
        appLocales(LocaleList.forLanguageTags("zh-Hans-CN"))
        syncAppLocalesFromSystem(context)
        assertEquals("zh-Hans-CN", AppLocalesStorageHelper.readLocales(context))
    }

    @Test
    fun coreLanguageListStartsWithAppLanguage() {
        var payload by mutableStateOf(emptyList<String>())
        compose.setContent {
            // 生产形态：App 语言 zh-Hans（资源按它落），系统语言偏好是 en-US、fr-FR。
            val config = Configuration(LocalConfiguration.current)
            config.setLocales(LocaleList.forLanguageTags("en-US,fr-FR"))
            CompositionLocalProvider(LocalConfiguration provides config) {
                payload = coreLanguageTags(appLanguageTag(), LocalConfiguration.current.locales)
            }
        }
        assertEquals(listOf("zh-Hans", "en-US", "fr-FR"), payload)
    }

    private fun appLocales(locales: LocaleList) {
        context.getSystemService(LocaleManager::class.java).setApplicationLocales(locales)
    }
}
