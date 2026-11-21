package app.wonton.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Compose
import app.wonton.ui.icons.WontonIcon

/** 输入框上方的编辑状态：说明在改哪条、一行摘要，叉放弃并恢复原草稿（ADR-0109 第 8 节）。 */
@Composable
internal fun ComposerEditNotice(summary: String, onCancel: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(
            start = WontonTheme.spacing.s16,
            end = WontonTheme.spacing.s8,
            bottom = WontonTheme.spacing.s4,
        ),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(
            WontonIcon.Compose,
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.iconSmall),
            tint = WontonTheme.colors.textSecondary.toColor(),
        )
        Column(Modifier.weight(1f).padding(start = WontonTheme.spacing.s8)) {
            Text(
                stringResource(R.string.composer_editing),
                style = WontonTheme.typography.caption,
                color = WontonTheme.colors.textSecondary.toColor(),
            )
            Text(
                summary,
                style = WontonTheme.typography.caption,
                color = WontonTheme.colors.textSecondary.toColor(),
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        WontonIconButton(WontonIcon.Close, stringResource(R.string.composer_edit_cancel), onCancel)
    }
}
