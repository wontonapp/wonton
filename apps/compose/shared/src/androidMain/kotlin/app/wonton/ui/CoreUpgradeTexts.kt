package app.wonton.ui

import androidx.compose.ui.graphics.vector.ImageVector
import app.wonton.client.coreUpgradeFailureSummary
import app.wonton.client.coreUpgradePhaseTitle
import app.wonton.protocol.CoreUpgradeCheck
import app.wonton.protocol.CoreUpgradeOffer
import app.wonton.protocol.CoreUpgradeStage
import app.wonton.protocol.CoreUpgradeStatus
import app.wonton.protocol.CoreUpgradeTarget
import app.wonton.protocol.CoreVersion
import app.wonton.protocol.Rgb
import app.wonton.protocol.ThemeColors
import app.wonton.ui.icons.CheckCircleFill
import app.wonton.ui.icons.ErrorFill
import app.wonton.ui.icons.Info
import app.wonton.ui.icons.WarningFill
import app.wonton.ui.icons.WontonIcon

private const val SHORT_DIGEST = 12

internal class StageLine(
    val title: String,
    val detail: String? = null,
    /** 没有图标时转圈。 */
    val icon: ImageVector? = null,
    val tint: (ThemeColors) -> Rgb = { it.textSecondary },
)

internal fun CoreUpgradeStage.line(target: CoreUpgradeTarget?): StageLine? {
    val upgrading = target?.let { "正在升级到 ${it.label()}" } ?: "正在升级"
    return when (this) {
        is CoreUpgradeStage.Idle -> null
        is CoreUpgradeStage.Requesting -> StageLine("正在发起升级")
        is CoreUpgradeStage.InProgress -> StageLine(upgrading, coreUpgradePhaseTitle(phase))
        is CoreUpgradeStage.Reconnecting -> StageLine(upgrading, "暂时连不上")
        is CoreUpgradeStage.Uncertain -> StageLine(
            "结果不确定",
            coreUpgradeFailureSummary(problem.reason),
            WontonIcon.WarningFill,
        ) { it.warning }
        is CoreUpgradeStage.Completed -> StageLine("已升级到 ${to.label()}", icon = WontonIcon.CheckCircleFill) {
            it.success
        }
        is CoreUpgradeStage.RolledBack -> StageLine(
            "已退回 ${to.label()}",
            coreUpgradeFailureSummary(problem.reason),
            WontonIcon.WarningFill,
        ) { it.warning }
        is CoreUpgradeStage.Failed -> StageLine(
            "没升级成，什么都没动",
            coreUpgradeFailureSummary(problem.reason),
            WontonIcon.ErrorFill,
        ) { it.danger }
        is CoreUpgradeStage.Canceled -> StageLine("已取消升级", icon = WontonIcon.Info)
    }
}

internal fun CoreUpgradeStatus.detailRows(
    offer: CoreUpgradeOffer?,
    stage: CoreUpgradeStage,
): List<Pair<String, String>> =
    listOf(
        "原因" to (stage.problem()?.message ?: check.problem()?.message),
        "渠道" to channel,
        "当前版本" to current.digest,
        "新版本" to (offer?.digest ?: run?.to?.digest),
        "会迁移的数据" to offer?.migrates?.takeIf { it.isNotEmpty() }?.joinToString("、"),
    ).mapNotNull { (title, value) -> value?.let { title to it } }

/** 没有升级在进行，可以看渠道上的版本。 */
internal fun CoreUpgradeStage.isSettled() = when (this) {
    is CoreUpgradeStage.Requesting, is CoreUpgradeStage.InProgress, is CoreUpgradeStage.Reconnecting -> false
    else -> true
}

private fun CoreUpgradeStage.problem() = when (this) {
    is CoreUpgradeStage.Uncertain -> problem
    is CoreUpgradeStage.RolledBack -> problem
    is CoreUpgradeStage.Failed -> problem
    else -> null
}

internal fun CoreUpgradeCheck.canCheck() = when (this) {
    is CoreUpgradeCheck.NotInstalled, is CoreUpgradeCheck.Pinned, is CoreUpgradeCheck.Other -> false
    else -> true
}

private fun CoreUpgradeCheck.problem() = when (this) {
    is CoreUpgradeCheck.DownloadFailed -> problem
    is CoreUpgradeCheck.Failed -> problem
    else -> null
}

internal fun CoreVersion.label() = version ?: digest?.let(::shortDigest) ?: "开发版"

private fun CoreUpgradeTarget.label() = version ?: shortDigest(digest)

internal fun CoreUpgradeOffer.label() = version ?: shortDigest(digest)

private fun shortDigest(digest: String) = digest.removePrefix("sha256:").take(SHORT_DIGEST)
