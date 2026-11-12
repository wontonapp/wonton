package app.wonton.ui

import android.content.Context
import androidx.appcompat.app.AppCompatDelegate
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.core.os.LocaleListCompat
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonTheme
import java.util.Locale
import org.xmlpull.v1.XmlPullParser

private const val LOCALES_CONFIG = "locales_config"
private const val ANDROID_NS = "http://schemas.android.com/apk/res/android"

/** 设置首页的“语言”行：右边是当前选择，点了弹出语言选择器（ADR-0043）。 */
@Composable
internal fun LanguageSettingsRow() {
    var choosing by remember { mutableStateOf(false) }
    WontonRow(
        stringResource(R.string.settings_language),
        languageValue(),
        showsForward = true,
    ) { choosing = true }
    if (choosing) LanguageSelection { choosing = false }
}

@Composable
private fun languageValue(): String {
    val current = currentLanguageTag()
    return if (current == null) stringResource(R.string.settings_language_system) else selfName(current)
}

/** 用户给 App 挑的语言（AppCompat 存的，Android 13 起和系统的“应用语言”是同一个值）；跟随系统时为 null。 */
private fun currentLanguageTag(): String? =
    AppCompatDelegate.getApplicationLocales().takeUnless { it.isEmpty }?.get(0)?.toLanguageTag()

@Composable
private fun LanguageSelection(onDismiss: () -> Unit) {
    val context = LocalContext.current
    val tags = remember(context) { supportedLanguageTags(context) }
    val current = currentLanguageTag()
    WontonDialog(
        stringResource(R.string.settings_language),
        onDismiss = onDismiss,
        actions = listOf(WontonDialogAction(stringResource(R.string.common_close), onClick = onDismiss)),
        detail = {
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                WontonRadioRow(
                    title = stringResource(R.string.settings_language_system),
                    selected = current == null,
                    enabled = true,
                    onSelect = {
                        AppCompatDelegate.setApplicationLocales(LocaleListCompat.getEmptyLocaleList())
                        onDismiss()
                    },
                )
                tags.forEach { tag ->
                    WontonRadioRow(
                        title = selfName(tag),
                        selected = tag == current,
                        enabled = true,
                        onSelect = {
                            AppCompatDelegate.setApplicationLocales(LocaleListCompat.forLanguageTags(tag))
                            onDismiss()
                        },
                    )
                }
            }
        },
    )
}

/** App 支持的语言读自构建生成的 locales_config.xml（androidApp/build.gradle.kts），以后加语言只加翻译文件。 */
private fun supportedLanguageTags(context: Context): List<String> {
    // locales_config 生成在 App 模块里，shared 的 R 看不见，运行时从 App 的资源里按名字取。
    val id = context.resources.getIdentifier(LOCALES_CONFIG, "xml", context.packageName)
    check(id != 0) { "locales_config.xml is missing: the androidApp generateLocalesConfig task did not run" }
    val parser = context.resources.getXml(id)
    val tags = buildList {
        while (parser.next() != XmlPullParser.END_DOCUMENT) {
            if (parser.eventType == XmlPullParser.START_TAG && parser.name == "locale") {
                add(parser.getAttributeValue(ANDROID_NS, "name"))
            }
        }
    }
    parser.close()
    return tags
}

/** 语言用它自己的话叫什么，首字母大写（English、简体中文、繁體中文、日本語）。 */
private fun selfName(tag: String): String {
    val locale = Locale.forLanguageTag(tag)
    return locale.getDisplayName(locale).replaceFirstChar { it.titlecase(locale) }
}
