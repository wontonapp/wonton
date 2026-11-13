package app.wonton.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import app.wonton.protocol.BotView
import app.wonton.protocol.ConfirmationRequester
import androidx.compose.ui.unit.dp
import app.wonton.protocol.ConfirmationState
import app.wonton.protocol.ConfirmationView
import app.wonton.protocol.Action
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Person
import app.wonton.ui.icons.WontonIcon

/** 展开时右侧箭头转向下的角度。 */
private const val EXPANDED_ROTATION = 90f

/**
 * 确认卡片行（ADR-0016）：会话输入栏上方和“等你确认”列表共用。收起是一行——发起者头像、标题、
 * 到期时间或结果；点开是完整卡片，能直接回答。
 */
@Composable
fun ConfirmationCardRow(
    view: ConfirmationView,
    bots: List<BotView>,
    plugins: PluginsState,
    onAction: (Action) -> Unit,
    modifier: Modifier = Modifier,
) {
    var expanded by remember(view.card.id) { mutableStateOf(false) }
    Column(
        modifier.fillMaxWidth()
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s12),
    ) {
        CardRowHeader(
            view,
            bots,
            plugins,
            secondary = null,
            trailing = rowLine(view),
            expanded,
            Modifier.clickable { expanded = !expanded },
        )
        if (expanded) {
            ConfirmationCardView(view, onAction)
        }
    }
}

/** 会话里输入栏上方的一行：显示最早到期的一张，多张时带“还有 N 张”，点开第一张的完整卡片。 */
@Composable
fun ConfirmationCardBar(
    views: List<ConfirmationView>,
    bots: List<BotView>,
    plugins: PluginsState,
    onAction: (Action) -> Unit,
    modifier: Modifier = Modifier,
) {
    val first = views.first()
    var expanded by remember(first.card.id) { mutableStateOf(false) }
    Column(
        modifier.fillMaxWidth()
            .padding(horizontal = WontonTheme.spacing.pageInset, vertical = WontonTheme.spacing.s8),
    ) {
        val more = views.size - 1
        CardRowHeader(
            first,
            bots,
            plugins,
            secondary = more.takeIf { it > 0 }
                ?.let { pluralStringResource(R.plurals.confirmations_more_cards, it, it) },
            trailing = rowLine(first),
            expanded,
            Modifier.clickable { expanded = !expanded },
        )
        if (expanded) {
            androidx.compose.foundation.lazy.LazyColumn(
                Modifier.heightIn(max = 400.dp),
            ) {
                items(views.size, key = { views[it].card.id }) { index ->
                    val selected = views[index]
                    if (index == 0) ConfirmationCardView(selected, onAction)
                    else ConfirmationCardRow(selected, bots, plugins, onAction)
                }
            }
        }
    }
}

/** 收起的卡片行：发起者头像、标题和一行次要信息，右侧箭头指示能不能展开。 */
@Composable
private fun CardRowHeader(
    view: ConfirmationView,
    bots: List<BotView>,
    plugins: PluginsState,
    secondary: String?,
    trailing: String?,
    expanded: Boolean,
    modifier: Modifier = Modifier,
) {
    val colors = WontonTheme.colors
    Row(
        modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
    ) {
        RequesterAvatar(view.card.requester, bots, plugins)
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
            Text(
                view.card.title,
                style = WontonTheme.typography.itemTitle,
                color = colors.textPrimary.toColor(),
            )
            secondary?.let {
                Text(
                    it,
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            trailing?.let {
                Text(
                    it,
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        Icon(
            WontonIcon.Forward,
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.iconSmall).rotate(if (expanded) EXPANDED_ROTATION else 0f),
            tint = colors.textSecondary.toColor(),
        )
    }
}

/** 按 requester 画头像：Bot 按 `userId` 在 bots 里找（ADR-0044），插件用插件图标；都不认识时用占位。 */
@Composable
private fun RequesterAvatar(requester: ConfirmationRequester?, bots: List<BotView>, plugins: PluginsState) {
    val size = WontonTheme.size.avatar
    when (requester) {
        is ConfirmationRequester.Bot -> {
            val view = bots.firstOrNull { it.userId == requester.userId }
            if (view != null) {
                BotAvatar(view, plugins, size)
            } else {
                Placeholder(size)
            }
        }
        is ConfirmationRequester.Plugin -> {
            val icon = plugins.viewModel.installed?.plugins?.firstOrNull { it.id == requester.pluginId }?.icon
            PluginIconImage(icon, size, plugins)
        }
        ConfirmationRequester.Other, null -> Placeholder(size)
    }
}

@Composable
private fun Placeholder(size: Dp) {
    Box(
        Modifier.size(size).background(WontonTheme.colors.surfaceSubtle.toColor(), CircleShape),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            WontonIcon.Person,
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.icon),
            tint = WontonTheme.colors.textSecondary.toColor(),
        )
    }
}

/** 收起行上显示的一句：等回答时是到期时刻，结束的是结果。 */
@Composable
private fun rowLine(view: ConfirmationView): String? = when (val state = view.card.state) {
    ConfirmationState.Pending ->
        ConfirmationRemaining(view.card.expiresAtMs.toLong())
    ConfirmationState.Running -> stringResource(R.string.confirmation_card_running)
    is ConfirmationState.Succeeded -> stringResource(R.string.confirmations_state_succeeded)
    is ConfirmationState.Failed -> state.message.takeIf { it.isNotEmpty() }
        ?.let { stringResource(R.string.confirmation_card_failed_result, it) }
        ?: stringResource(R.string.confirmations_state_failed)
    is ConfirmationState.Uncertain -> state.message.takeIf { it.isNotEmpty() }
        ?.let { stringResource(R.string.confirmation_card_uncertain_result, it) }
        ?: stringResource(R.string.confirmations_state_uncertain)
    ConfirmationState.Rejected -> stringResource(R.string.confirmation_card_rejected)
    ConfirmationState.Expired -> stringResource(R.string.confirmation_card_expired)
}
