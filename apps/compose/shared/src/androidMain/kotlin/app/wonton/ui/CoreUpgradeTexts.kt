package app.wonton.ui

import android.icu.text.ListFormatter
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.CoreUpgradeCheck
import app.wonton.protocol.CoreUpgradeFailure
import app.wonton.protocol.CoreUpgradePhase
import app.wonton.protocol.CoreUpgradeProblem
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

@Composable
internal fun CoreUpgradeStage.line(target: CoreUpgradeTarget?): StageLine? {
    val upgrading = target?.let { stringResource(R.string.core_upgrade_upgrading_to, it.label) }
        ?: stringResource(R.string.core_upgrade_upgrading)
    return when (this) {
        is CoreUpgradeStage.Idle -> null
        is CoreUpgradeStage.Requesting -> StageLine(stringResource(R.string.core_upgrade_requesting_upgrade))
        is CoreUpgradeStage.InProgress -> StageLine(upgrading, stringResource(phaseTitle(phase)))
        is CoreUpgradeStage.Reconnecting -> StageLine(upgrading, stringResource(R.string.core_upgrade_reconnecting))
        is CoreUpgradeStage.Uncertain -> StageLine(
            stringResource(R.string.core_upgrade_uncertain),
            stringResource(failureTitle(problem.reason)),
            WontonIcon.WarningFill,
        ) { it.warning }
        is CoreUpgradeStage.Completed -> StageLine(
            stringResource(R.string.core_upgrade_completed_to, to.label),
            icon = WontonIcon.CheckCircleFill,
        ) {
            it.success
        }
        is CoreUpgradeStage.RolledBack -> StageLine(
            stringResource(R.string.core_upgrade_rolled_back_to, to.label),
            stringResource(failureTitle(problem.reason)),
            WontonIcon.WarningFill,
        ) { it.warning }
        is CoreUpgradeStage.Failed -> StageLine(
            stringResource(R.string.core_upgrade_failed_no_changes),
            stringResource(failureTitle(problem.reason)),
            WontonIcon.ErrorFill,
        ) { it.danger }
        is CoreUpgradeStage.Canceled -> StageLine(
            stringResource(R.string.core_upgrade_canceled), icon = WontonIcon.Info,
        )
    }
}

@Composable
internal fun CoreUpgradeStatus.detailRows(
    offer: CoreUpgradeOffer?,
    stage: CoreUpgradeStage,
): List<Pair<String, String>> = listOf(
    stringResource(R.string.core_upgrade_detail_reason) to (stage.problem()?.message ?: check.problem()?.message),
    stringResource(R.string.core_upgrade_detail_channel) to channel,
    stringResource(R.string.core_upgrade_current_version) to current.digest,
    stringResource(R.string.core_upgrade_detail_new_version) to (offer?.digest ?: run?.to?.digest),
    stringResource(R.string.core_upgrade_detail_migrates) to offer?.migrates?.takeIf { it.isNotEmpty() }
        ?.let { ListFormatter.getInstance(LocalConfiguration.current.locales[0]).format(it) },
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

private val CoreUpgradeTarget.label: String
    get() = version ?: shortDigest(digest)

private fun CoreUpgradeCheck.problem(): CoreUpgradeProblem? = when (val c = this) {
    is CoreUpgradeCheck.DownloadFailed -> c.problem
    is CoreUpgradeCheck.Failed -> c.problem
    else -> null
}

internal val CoreUpgradeOffer.label: String
    get() = version ?: shortDigest(digest)

internal val CoreVersion.label: String
    @Composable get() = version ?: digest?.let(::shortDigest) ?: stringResource(R.string.core_upgrade_dev_version)

private fun shortDigest(digest: String) = digest.removePrefix("sha256:").take(SHORT_DIGEST)

/** 升级的阶段，给壳显示。 */
internal fun phaseTitle(phase: CoreUpgradePhase): Int = when (phase) {
    CoreUpgradePhase.PULLING -> R.string.core_upgrade_phase_pulling
    CoreUpgradePhase.BACKING_UP -> R.string.core_upgrade_phase_backing_up
    CoreUpgradePhase.SWITCHING -> R.string.core_upgrade_phase_switching
    CoreUpgradePhase.STARTING -> R.string.core_upgrade_phase_starting
    CoreUpgradePhase.OBSERVING -> R.string.core_upgrade_phase_observing
    CoreUpgradePhase.ROLLING_BACK -> R.string.core_upgrade_phase_rolling_back
    CoreUpgradePhase.OTHER -> R.string.core_upgrade_phase_other
}

/** 失败原因的资源，显示在升级页上；核心写的详细原因放详情。 */
internal fun failureTitle(reason: CoreUpgradeFailure): Int = when (reason) {
    CoreUpgradeFailure.TRANSPORT_UNAVAILABLE -> R.string.problem_transport_unavailable_title
    CoreUpgradeFailure.UNAUTHORIZED -> R.string.problem_unauthorized_title
    CoreUpgradeFailure.CHANGED -> R.string.core_upgrade_failure_changed
    CoreUpgradeFailure.NOT_READY -> R.string.core_upgrade_failure_not_ready
    CoreUpgradeFailure.BUSY -> R.string.core_upgrade_failure_busy
    CoreUpgradeFailure.TOO_LATE -> R.string.core_upgrade_failure_too_late
    CoreUpgradeFailure.REGISTRY_UNAVAILABLE -> R.string.core_upgrade_failure_registry_unavailable
    CoreUpgradeFailure.NO_SPACE -> R.string.core_upgrade_failure_no_space
    CoreUpgradeFailure.INIT_FAILED -> R.string.core_upgrade_failure_init_failed
    CoreUpgradeFailure.UNHEALTHY -> R.string.core_upgrade_failure_unhealthy
    CoreUpgradeFailure.CRASHED -> R.string.core_upgrade_failure_crashed
    CoreUpgradeFailure.TIMED_OUT -> R.string.core_upgrade_failure_timed_out
    CoreUpgradeFailure.UPDATER_FAILED -> R.string.core_upgrade_failure_updater_failed
    CoreUpgradeFailure.UPSTREAM -> R.string.core_upgrade_failure_upstream
    CoreUpgradeFailure.PROTOCOL, CoreUpgradeFailure.OTHER -> R.string.core_upgrade_failure_protocol
}
