// 核心升级页兼容块（ADR-0085）的假数据和预览，和 PreviewData.kt 一样只进测试源集，不连核心、不加载 .so。
@file:Suppress("MagicNumber")

package app.wonton.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import app.wonton.protocol.CoreUpgradeBreakingPlugin
import app.wonton.protocol.CoreUpgradeBreakingReason
import app.wonton.protocol.CoreUpgradeCompat
import app.wonton.protocol.CoreUpgradeCompatCategory
import app.wonton.protocol.CoreUpgradeFailure
import app.wonton.protocol.CoreUpgradeProblem
import app.wonton.ui.design.WontonTheme

/** 升级卡片里那块的页面内边距，和 CoreUpgradeContent 一致。 */
private val PAGE_PADDING = WontonTheme.spacing.pageInset

private fun previewBreaking() = CoreUpgradeCompat.Breaking(
    plugins = listOf(
        CoreUpgradeBreakingPlugin(
            id = "app.immich.photos",
            reasons = listOf(
                CoreUpgradeBreakingReason(category = CoreUpgradeCompatCategory.SPEC_UNSUPPORTED, raw = "0.13"),
            ),
        ),
        CoreUpgradeBreakingPlugin(
            id = "app.copyparty.files",
            reasons = listOf(
                CoreUpgradeBreakingReason(category = CoreUpgradeCompatCategory.UNKNOWN_FEATURE, raw = "x-wonton.tabs"),
            ),
        ),
    ),
)

private fun previewApps() = listOf(
    CompatApp("app.immich.photos", "Photos (Immich)", null),
    CompatApp("app.copyparty.files", "Files (Copyparty)", null),
)

/** 升级会新失效的应用：警示色标题 + 图标列表（图标恒空，走占位分支）。 */
@Preview(name = "升级兼容（会失效）", locale = "en", widthDp = 390, heightDp = 180)
@Composable
internal fun CoreUpgradeCompatBreakingPreview() {
    PreviewFrame {
        Column(Modifier.fillMaxWidth().padding(PAGE_PADDING)) {
            CompatSection(
                compat = previewBreaking(),
                apps = previewApps(),
                refreshing = false,
                onRetry = {},
                icon = { PluginIconImage(it, WontonTheme.size.taskIcon, imageOrNull = { null }) },
            )
        }
    }
}

/** 没查成哪些应用会受影响：轻提示一行，点一下重查。 */
@Preview(name = "升级兼容（没查成）", locale = "en", widthDp = 390, heightDp = 120)
@Composable
internal fun CoreUpgradeCompatFailedPreview() {
    PreviewFrame {
        Column(Modifier.fillMaxWidth().padding(PAGE_PADDING)) {
            CompatSection(
                compat = CoreUpgradeCompat.Failed(
                    problem = CoreUpgradeProblem(
                        reason = CoreUpgradeFailure.UPSTREAM,
                        message = "Docker reported an error.",
                    ),
                ),
                apps = emptyList(),
                refreshing = false,
                onRetry = {},
                icon = { PluginIconImage(it, WontonTheme.size.taskIcon, imageOrNull = { null }) },
            )
        }
    }
}
