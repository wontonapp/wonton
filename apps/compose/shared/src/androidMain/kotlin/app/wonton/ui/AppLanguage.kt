package app.wonton.ui

import android.app.LocaleManager
import android.content.Context
import android.os.Build
import android.os.LocaleList
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import androidx.core.app.AppLocalesStorageHelper

/**
 * App 实际在用的语言标签（资源落到的那种，如 zh-Hans）。系统语言是 zh-TW 时这里是 zh-Hant：
 * 用户要的语言和资源实际用的语言不一样，要的是后者。
 */
@Composable
fun appLanguageTag(): String = stringResource(R.string.language_tag)

/** 给核心的语言列表：App 在用的语言在最前，其后是系统的语言偏好，忽略大小写去重，保持顺序。 */
fun coreLanguageTags(shown: String, system: LocaleList): List<String> =
    (listOf(shown) + system.toLanguageTags().split(","))
        .filter { it.isNotBlank() }
        .distinctBy { it.lowercase() }

/** Android 13 起系统才接管“应用语言”（LocaleManager），存储对齐只在这一档做。 */
private const val APP_LOCALES_SDK = 33

/**
 * Android 13+ 以系统 LocaleManager 为准对齐 AppCompat 的存储。系统设置（或 `cmd locale`）里改的
 * “应用语言”只写 LocaleManager，AppCompat 的存储还是空的；Activity 首次 attach 时它的同步见存储
 * 为空就把空列表写回 LocaleManager，抹掉用户刚选的语言（实测：装好后先设 zh-Hans-CN 再首启，语言
 * 被清成空、界面落回英文）。启动时先把系统值种进存储，这条同步就只是写回同一个值。系统把 App
 * 语言清成空时不种旧值，让 AppCompat 的同步把空写回去，两边一致。更早的系统没有这个口径，系统
 * 设置里也没有“应用语言”，不动。
 */
fun syncAppLocalesFromSystem(context: Context) {
    if (Build.VERSION.SDK_INT < APP_LOCALES_SDK) return
    val framework = context.getSystemService(LocaleManager::class.java).applicationLocales
    if (framework.isEmpty) return
    val tags = framework.toLanguageTags()
    if (AppLocalesStorageHelper.readLocales(context) != tags) {
        AppLocalesStorageHelper.persistLocales(context, tags)
    }
}
