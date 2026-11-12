package app.wonton.ui

import android.os.LocaleList
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource

/**
 * App 实际在用的语言标签（资源落到的那种，如 zh-Hans）。系统语言是 zh-TW 时这里是 zh-Hant：
 * 用户要的语言和资源实际用的语言不一样，要的是后者。
 */
@Composable
fun appLanguageTag(): String = stringResource(R.string.language_tag)

/** 给核心的语言列表：App 在用的语言在最前，其后是系统的语言偏好，忽略大小写去重，保持顺序。 */
internal fun coreLanguageTags(shown: String, system: LocaleList): List<String> =
    (listOf(shown) + system.toLanguageTags().split(","))
        .filter { it.isNotBlank() }
        .distinctBy { it.lowercase() }
