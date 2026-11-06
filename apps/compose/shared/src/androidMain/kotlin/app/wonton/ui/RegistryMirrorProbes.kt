package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.style.TextAlign
import app.wonton.protocol.RegistryFailure
import app.wonton.protocol.RegistryProbe
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonRadioRow
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor

private const val RESULT_WEIGHT = 0.65f

@Composable
internal fun RegistryChoiceRow(
    title: String,
    selected: Boolean,
    enabled: Boolean,
    probe: RegistryProbe?,
    onSelect: () -> Unit,
) {
    Column {
        WontonRadioRow(
            title, selected, enabled, onSelect, probe.registryLabel(),
            valueTint = WontonTheme.colors.danger.takeIf { probe?.problem != null },
        )
        ProbeDetail(probe)
    }
}

@Composable
internal fun RegistryProbeRow(title: String, probe: RegistryProbe?) {
    WontonGroup {
        Row(
            Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target).padding(WontonTheme.spacing.s12),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                title, Modifier.weight(1f),
                style = WontonTheme.typography.body, color = WontonTheme.colors.textPrimary.toColor(),
            )
            Text(
                probe.registryLabel(), Modifier.weight(RESULT_WEIGHT),
                style = WontonTheme.typography.supporting.copy(textAlign = TextAlign.End),
                color = (if (probe?.problem != null) WontonTheme.colors.danger
                    else WontonTheme.colors.textSecondary).toColor(),
            )
        }
        ProbeDetail(probe)
    }
}

@Composable
private fun ProbeDetail(probe: RegistryProbe?) {
    probe?.problem?.let {
        Column(Modifier.padding(horizontal = WontonTheme.spacing.s12)) {
            ProblemDetail(
                it.message, tabText("详情", "Details"),
                Modifier.heightIn(min = WontonTheme.size.target),
            )
        }
    }
}

@Composable
private fun RegistryProbe?.registryLabel(): String {
    if (this == null) return tabText("不确定", "Unknown")
    return when (problem?.reason) {
        RegistryFailure.UNREACHABLE -> tabText("连不上", "Unreachable")
        RegistryFailure.REJECTED -> tabText("被拒绝", "Rejected")
        RegistryFailure.DIGEST_MISMATCH -> tabText("摘要不符", "Digest mismatch")
        RegistryFailure.OTHER -> tabText("其他错误", "Other failure")
        null -> latencyMs?.let { "$it ms" } ?: tabText("可用", "Usable")
    }
}
