package app.wonton.ui.design
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import app.wonton.protocol.StepNav
import app.wonton.ui.R
@Composable
fun StepCardFrame(
    nav: StepNav,
    title: String,
    primaryText: String,
    primaryBusy: Boolean,
    onBack: () -> Unit,
    onSkip: () -> Unit,
    onPrimary: () -> Unit,
    modifier: Modifier = Modifier,
    reject: WontonDialogAction? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(
        modifier.fillMaxWidth().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
    ) {
        StepCardHeader(nav, onBack, onSkip)
        Text(
            title,
            modifier = Modifier.semantics { heading() },
            style = WontonTheme.typography.itemTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        content()
        StepCardFooter(nav, primaryText, primaryBusy, onPrimary, reject)
    }
}
@Composable
private fun StepCardHeader(nav: StepNav, onBack: () -> Unit, onSkip: () -> Unit) {
    val stepDescription = stringResource(R.string.design_step_progress, nav.position.toInt(), nav.total.toInt())
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            "${nav.position}/${nav.total}",
            modifier = Modifier.weight(1f).clearAndSetSemantics { contentDescription = stepDescription },
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        WontonButton(
            stringResource(R.string.design_step_back),
            onBack,
            modifier = Modifier.weight(1f),
            kind = WontonButtonKind.SECONDARY,
            enabled = nav.canBack,
        )
        WontonButton(
            stringResource(R.string.design_skip),
            onSkip,
            modifier = Modifier.weight(1f),
            kind = WontonButtonKind.SECONDARY,
            enabled = nav.canSkip,
        )
    }
}
@Composable
private fun StepCardFooter(
    nav: StepNav,
    primaryText: String,
    primaryBusy: Boolean,
    onPrimary: () -> Unit,
    reject: WontonDialogAction?,
) {
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (primaryBusy) {
                val busyDescription = stringResource(R.string.design_busy)
                CircularProgressIndicator(
                    modifier = Modifier.size(WontonTheme.size.iconSmall)
                        .semantics { contentDescription = busyDescription },
                    color = WontonTheme.colors.textPrimary.toColor(),
                    strokeWidth = WontonTheme.size.hairline,
                )
            }
            WontonButton(primaryText, onPrimary, Modifier.weight(1f), enabled = nav.canNext)
        }
        reject?.let {
            WontonButton(it.title, it.onClick, Modifier.fillMaxWidth(), kind = it.kind)
        }
    }
}
