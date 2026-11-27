package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.AgentModelsAction
import app.wonton.protocol.AgentModelsViewModel
import app.wonton.protocol.ModelProviderRow
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonSearchField
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Check
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.WontonIcon

private const val EXPANDED_DEGREES = 90f

/** 模型页的内容：在用的模型、上一个动作没成的轻提示、服务商。外面给滚动和间距。 */
@Composable
internal fun AgentModelsList(view: AgentModelsViewModel, send: (AgentModelsAction) -> Unit) {
    CurrentModelCard(view, send)
    view.actionProblem?.let { problem -> key(problem) { ProblemRow(problem.problem) } }
    if (view.recommended.isNotEmpty()) ProviderGroup(view.recommended, send)
    MoreProviders(view, send)
}

/** 推荐以外的服务商：收在“更多”里，展开后能搜。没有推荐时直接摊开。 */
@Composable
private fun MoreProviders(view: AgentModelsViewModel, send: (AgentModelsAction) -> Unit) {
    val collapsible = view.recommended.isNotEmpty()
    var expanded by rememberSaveable { mutableStateOf(false) }
    // 输入法组字时文字要立刻回到输入框，等核心绕一圈会丢字；核心拿到的是同一份。
    var search by remember { mutableStateOf(view.search) }
    val open = expanded || !collapsible || view.search.isNotEmpty()
    if (!open && view.more.isEmpty()) return
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        if (collapsible) MoreHeader(open) { expanded = !open }
        if (!open) return@Column
        WontonSearchField(
            search,
            {
                search = it
                send(AgentModelsAction.SearchModelProviders(it))
            },
            stringResource(R.string.agent_models_search),
        )
        when {
            view.more.isNotEmpty() -> ProviderGroup(view.more, send)
            else -> Text(
                stringResource(
                    if (view.search.isEmpty()) R.string.agent_models_no_providers else R.string.agent_models_no_match,
                ),
                style = WontonTheme.typography.supporting,
                color = WontonTheme.colors.textSecondary.toColor(),
            )
        }
    }
}

@Composable
private fun MoreHeader(open: Boolean, toggle: () -> Unit) {
    val tint = WontonTheme.colors.textSecondary.toColor()
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = toggle)
            .padding(horizontal = WontonTheme.spacing.s4),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(stringResource(R.string.agent_models_more), style = WontonTheme.typography.supporting, color = tint)
        Icon(
            WontonIcon.Forward,
            // 装饰：标签文字在左边，箭头只表示能展开。
            contentDescription = null,
            Modifier.size(WontonTheme.size.iconTiny).rotate(if (open) EXPANDED_DEGREES else 0f),
            tint = tint,
        )
    }
}

@Composable
private fun ProviderGroup(rows: List<ModelProviderRow>, send: (AgentModelsAction) -> Unit) {
    WontonGroup {
        rows.forEachIndexed { index, row ->
            key(row.id) {
                if (index > 0) WontonSeparator()
                ProviderRow(row) { send(AgentModelsAction.OpenModelProvider(row.id)) }
            }
        }
    }
}

/** 一家服务商：名字，接好的带账号（或 key 尾号）和勾。 */
@Composable
private fun ProviderRow(row: ModelProviderRow, onClick: () -> Unit) {
    val colors = WontonTheme.colors
    val connected = stringResource(R.string.agent_models_connected)
    Row(
        Modifier.fillMaxWidth().heightIn(min = WontonTheme.size.target)
            .clickable(role = Role.Button, onClick = onClick)
            .then(if (row.connected) Modifier.semantics { stateDescription = connected } else Modifier)
            .padding(horizontal = WontonTheme.spacing.s16, vertical = WontonTheme.spacing.s12),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(row.name, Modifier.weight(1f), style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
        row.account?.takeIf { row.connected }?.let {
            Text(
                it, style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor(),
                maxLines = 1, overflow = TextOverflow.Ellipsis,
            )
        }
        if (row.connected) {
            // 装饰：“已接好”由整行的状态朗读。
            Icon(WontonIcon.Check, null, Modifier.size(WontonTheme.size.iconSmall), tint = colors.success.toColor())
        }
        // 装饰：箭头只表示这一行能点。
        Icon(WontonIcon.Forward, null, Modifier.size(WontonTheme.size.iconSmall), tint = colors.textSecondary.toColor())
    }
}
