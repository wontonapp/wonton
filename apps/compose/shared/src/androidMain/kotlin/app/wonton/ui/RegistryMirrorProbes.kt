package app.wonton.ui

import android.icu.text.MeasureFormat
import android.icu.util.Measure
import android.icu.util.MeasureUnit
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
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.res.stringResource
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
                it.message, stringResource(R.string.common_details),
                Modifier.heightIn(min = WontonTheme.size.target),
            )
        }
    }
}

@Composable
private fun RegistryProbe?.registryLabel(): String {
    if (this == null) return stringResource(R.string.registry_mirror_probe_unknown)
    val latency = latencyMs
    return when (problem?.reason) {
        RegistryFailure.UNREACHABLE -> stringResource(R.string.registry_mirror_probe_unreachable)
        RegistryFailure.REJECTED -> stringResource(R.string.registry_mirror_probe_rejected)
        RegistryFailure.DIGEST_MISMATCH -> stringResource(R.string.registry_mirror_probe_digest_mismatch)
        RegistryFailure.OTHER -> stringResource(R.string.registry_mirror_probe_other)
        null -> if (latency != null) {
            MeasureFormat.getInstance(LocalConfiguration.current.locales[0], MeasureFormat.FormatWidth.SHORT)
                .format(Measure(latency.toLong(), MeasureUnit.MILLISECOND))
        } else {
            stringResource(R.string.registry_mirror_probe_available)
        }
    }
}
