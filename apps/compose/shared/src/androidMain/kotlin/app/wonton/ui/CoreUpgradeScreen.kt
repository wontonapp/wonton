package app.wonton.ui

import android.text.format.Formatter
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import app.wonton.protocol.Action
import app.wonton.protocol.CoreUpgradeCheck
import app.wonton.protocol.CoreUpgradeOffer
import app.wonton.protocol.CoreUpgradeProblem
import app.wonton.protocol.CoreUpgradeProgress
import app.wonton.protocol.CoreUpgradeStage
import app.wonton.protocol.CoreUpgradeStatus
import app.wonton.protocol.CoreUpgradeTarget
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.WontonIcon

private const val DETAILS_OPEN_DEGREES = 90f
private val SPINNER_STROKE = 2.dp

/** 升级家里的服务器（ADR-0025）：进展和结论都在核心，这里渲染，并把主人的操作发回去。 */
@Composable
fun CoreUpgradePage(plugins: PluginsState, onBack: () -> Unit) {
    val upgrade = plugins.viewModel.coreUpgrade
    LaunchedEffect(Unit) { plugins.refresh(Action.RefreshCoreUpgrade(false)) }
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.core_upgrade_title), onBack) }) {
        val status = upgrade.status
        val problem = upgrade.problem
        when {
            status != null -> CoreUpgradeContent(plugins, status)
            problem != null -> ProblemView(
                Problem(titleRes = failureTitle(problem.reason), detail = problem.message),
            ) { plugins.send(Action.RefreshCoreUpgrade(true)) }
            else -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator() }
        }
    }
}

@Composable
private fun CoreUpgradeContent(plugins: PluginsState, status: CoreUpgradeStatus) {
    val upgrade = plugins.viewModel.coreUpgrade
    val colors = WontonTheme.colors
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        upgrade.problem?.let {
            Text(it.message, style = WontonTheme.typography.supporting, color = colors.danger.toColor())
        }
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                stringResource(R.string.core_upgrade_current_version),
                style = WontonTheme.typography.body,
                color = colors.textSecondary.toColor(),
            )
            Text(status.current.label, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
        }
        StageCard(plugins, upgrade.stage, status.run?.to)
        if (upgrade.stage.isSettled()) CheckSection(plugins, status.check)
        Details(status, upgrade.offer, upgrade.stage)
    }
}

@Composable
private fun CheckSection(plugins: PluginsState, check: CoreUpgradeCheck) {
    val upgrade = plugins.viewModel.coreUpgrade
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        val offer = upgrade.offer
        if (offer != null) OfferCard(plugins, offer, upgrade.offerSkipped) else CheckRow(check)
        if (check.canCheck()) {
            WontonButton(
                if (upgrade.refreshing) stringResource(R.string.core_upgrade_checking)
                else stringResource(R.string.core_upgrade_check_for_update),
                onClick = { plugins.send(Action.RefreshCoreUpgrade(true)) },
                modifier = Modifier.fillMaxWidth(),
                kind = WontonButtonKind.SECONDARY,
                enabled = !upgrade.refreshing,
            )
        }
    }
}

@Composable
private fun OfferCard(plugins: PluginsState, offer: CoreUpgradeOffer, skipped: Boolean) {
    val colors = WontonTheme.colors
    var confirming by remember(offer.digest) { mutableStateOf(false) }
    Section {
        Text(
            if (confirming) stringResource(R.string.core_upgrade_confirm_version, offer.label)
            else stringResource(R.string.core_upgrade_new_version, offer.label),
            style = WontonTheme.typography.itemTitle,
            color = colors.textPrimary.toColor(),
        )
        if (confirming && offer.migrates.isNotEmpty()) {
            Text(
                stringResource(R.string.core_upgrade_migrates_warning),
                style = WontonTheme.typography.body,
                color = colors.danger.toColor(),
            )
        }
        if (!confirming && skipped) {
            Text(
                stringResource(R.string.core_upgrade_skipped),
                style = WontonTheme.typography.supporting,
                color = colors.textSecondary.toColor(),
            )
        }
        if (confirming) {
            FullWidthButton(stringResource(R.string.core_upgrade_confirm_upgrade)) {
                plugins.send(Action.StartCoreUpgrade(offer.digest))
            }
            FullWidthButton(stringResource(R.string.core_upgrade_decline_upgrade), WontonButtonKind.SECONDARY) {
                confirming = false
            }
        } else {
            FullWidthButton(stringResource(R.string.core_upgrade_upgrade)) { confirming = true }
            if (!skipped) {
                FullWidthButton(stringResource(R.string.core_upgrade_skip_version), WontonButtonKind.SECONDARY) {
                    plugins.send(Action.SkipCoreUpgrade(offer.digest))
                }
            }
        }
    }
}

@Composable
private fun FullWidthButton(
    text: String,
    kind: WontonButtonKind = WontonButtonKind.PRIMARY,
    onClick: () -> Unit,
) {
    WontonButton(text, onClick, Modifier.fillMaxWidth(), kind)
}

@Composable
private fun Section(content: @Composable () -> Unit) {
    Column(
        Modifier.fillMaxWidth(),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) { content() }
}

/** 这次或上次升级走到哪了。 */
@Composable
private fun StageCard(plugins: PluginsState, stage: CoreUpgradeStage, target: CoreUpgradeTarget?) {
    val line = stage.line(target) ?: return
    val colors = WontonTheme.colors
    Section {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        ) {
            if (line.icon != null) {
                Icon(
                    line.icon,
                    // 装饰：状态由右边的文字说。
                    contentDescription = null,
                    Modifier.size(WontonTheme.size.icon),
                    tint = line.tint(colors).toColor(),
                )
            } else {
                CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = SPINNER_STROKE)
            }
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                Text(line.title, style = WontonTheme.typography.itemTitle, color = colors.textPrimary.toColor())
                line.detail?.let {
                    Text(it, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
                }
            }
        }
        if (stage is CoreUpgradeStage.InProgress && stage.canCancel) {
            FullWidthButton(stringResource(R.string.core_upgrade_cancel_upgrade), WontonButtonKind.DANGER) {
                plugins.send(Action.CancelCoreUpgrade)
            }
        }
        if (stage is CoreUpgradeStage.Uncertain) {
            FullWidthButton(stringResource(R.string.core_upgrade_check_again), WontonButtonKind.SECONDARY) {
                plugins.send(Action.RefreshCoreUpgrade(false))
            }
        }
    }
}

/** 没有可以升级的版本时，渠道上的情况。 */
@Composable
private fun CheckRow(check: CoreUpgradeCheck) {
    val colors = WontonTheme.colors
    val secondary: @Composable (String) -> Unit = {
        Text(it, style = WontonTheme.typography.body, color = colors.textSecondary.toColor())
    }
    when (check) {
        is CoreUpgradeCheck.NotInstalled -> secondary(stringResource(R.string.core_upgrade_check_not_installed))
        is CoreUpgradeCheck.Pinned -> secondary(stringResource(R.string.core_upgrade_check_pinned))
        is CoreUpgradeCheck.NotChecked, is CoreUpgradeCheck.Checking ->
            secondary(stringResource(R.string.core_upgrade_check_checking))
        is CoreUpgradeCheck.UpToDate -> secondary(stringResource(R.string.core_upgrade_check_up_to_date))
        is CoreUpgradeCheck.Downloading -> Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            secondary(stringResource(R.string.core_upgrade_check_downloading))
            check.progress?.takeIf { it.totalBytes > 0u }?.let { DownloadProgress(it) }
        }
        is CoreUpgradeCheck.DownloadFailed -> FailureRow(check.problem)
        is CoreUpgradeCheck.Failed -> FailureRow(check.problem)
        is CoreUpgradeCheck.Ready -> Unit
        is CoreUpgradeCheck.Other -> secondary(stringResource(R.string.core_upgrade_check_other))
    }
}

@Composable
private fun FailureRow(problem: CoreUpgradeProblem) {
    Text(
        stringResource(failureTitle(problem.reason)),
        style = WontonTheme.typography.body,
        color = WontonTheme.colors.danger.toColor(),
    )
}

@Composable
private fun DownloadProgress(progress: CoreUpgradeProgress) {
    val context = LocalContext.current
    val size = { bytes: ULong -> Formatter.formatFileSize(context, bytes.toLong()) }
    LinearProgressIndicator(
        progress = { (progress.doneBytes.toDouble() / progress.totalBytes.toDouble()).toFloat() },
        modifier = Modifier.fillMaxWidth(),
    )
    Text(
        "${size(progress.doneBytes)} / ${size(progress.totalBytes)}",
        style = WontonTheme.typography.caption,
        color = WontonTheme.colors.textSecondary.toColor(),
    )
}

@Composable
private fun Details(status: CoreUpgradeStatus, offer: CoreUpgradeOffer?, stage: CoreUpgradeStage) {
    val colors = WontonTheme.colors
    var open by remember { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(
            Modifier.fillMaxWidth()
                .heightIn(min = WontonTheme.size.target)
                .clickable(role = Role.Button) { open = !open },
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                stringResource(R.string.common_details),
                Modifier.weight(1f),
                style = WontonTheme.typography.body,
                color = colors.textSecondary.toColor(),
            )
            Icon(
                WontonIcon.Forward,
                // 装饰：整行可点，箭头只表示能展开。
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconSmall).rotate(if (open) DETAILS_OPEN_DEGREES else 0f),
                tint = colors.textSecondary.toColor(),
            )
        }
        if (open) {
            status.detailRows(offer, stage).forEach { (title, value) ->
                Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                    Text(title, style = WontonTheme.typography.caption, color = colors.textSecondary.toColor())
                    SelectionContainer {
                        Text(value, style = WontonTheme.typography.supporting, color = colors.textPrimary.toColor())
                    }
                }
            }
        }
    }
}
