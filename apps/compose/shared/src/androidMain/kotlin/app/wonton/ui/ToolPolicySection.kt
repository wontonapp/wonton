package app.wonton.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import app.wonton.protocol.BotSwitch
import app.wonton.protocol.CoreException
import app.wonton.protocol.OpenWindow
import app.wonton.protocol.ToolAccess
import app.wonton.protocol.ToolPolicyEntry
import app.wonton.protocol.ToolPolicyView
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonSwitch
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private const val EXPANDED_DEGREES = 90f
private const val DESCRIPTION_LINES = 2

private val ToolPolicyView.isEmpty: Boolean
    get() = tools.isEmpty() && toolsError == null && always.isEmpty() && allowAll.isEmpty() &&
        windows.isEmpty() && problem == null

/** 插件的工具权限（ADR-0024）：放行中的时段、始终允许、一律允许、哪些工具算只读。开关打开时不再弹确认，风险写在旁边。 */
@Composable
fun ToolPolicySection(plugins: PluginsState, id: String, version: String) {
    val scope = rememberCoroutineScope()
    var policy by remember(id) { mutableStateOf<ToolPolicyView?>(null) }
    var problem by remember(id) { mutableStateOf<Problem?>(null) }
    var busy by remember(id) { mutableStateOf(false) }
    var attempt by remember(id) { mutableIntStateOf(0) }
    LaunchedEffect(id, version, attempt) {
        try {
            policy = plugins.core.toolPolicy(id)
            problem = null
        } catch (failure: CoreException) {
            problem = failure.problem
        }
    }
    // 到点的时段不再生效，这一行要跟着消失。
    LaunchedEffect(policy?.windows) {
        val next = policy?.windows?.minOfOrNull { it.expiresAtMs.toLong() } ?: return@LaunchedEffect
        delay(next - System.currentTimeMillis())
        attempt++
    }
    val change: (suspend () -> ToolPolicyView) -> Unit = { call ->
        busy = true
        problem = null
        scope.launch {
            try {
                policy = call()
            } catch (failure: CoreException) {
                problem = failure.problem
                // 请求可能已经到了核心，开关以核心现在的为准。
                runCatching { plugins.core.toolPolicy(id) }.onSuccess { policy = it }
            }
            busy = false
        }
    }
    val current = policy
    when {
        current == null && problem == null ->
            CircularProgressIndicator(Modifier.size(WontonTheme.size.icon), strokeWidth = WontonTheme.size.hairline)
        current == null -> Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            problem?.let { ProblemNotice(it) }
            WontonButton("重试", { attempt++ }, kind = WontonButtonKind.SECONDARY)
        }
        !current.isEmpty || problem != null -> Column(
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            current.problem?.let {
                ProblemNotice(Problem("放宽的设置读不出来", "现在每次调用都会问你，这里也改不了。", it))
            }
            problem?.let { ProblemNotice(it) }
            val enabled = !busy && current.problem == null
            OpenWindows(current.windows, !busy) { card -> change { plugins.core.stopToolWindow(id, card) } }
            BotSwitches(
                "始终允许",
                "这个 Bot 用这个插件的工具不再问，包括会改数据的",
                current.always,
                enabled,
            ) { caller, on -> change { plugins.core.setToolAlways(id, caller, on) } }
            BotSwitches(
                "一律允许",
                "这个 Bot 用所有插件的工具都不再问，以后装的也算。提示注入能让它不经确认改数据、删东西、往外发内容",
                current.allowAll,
                enabled,
            ) { caller, on -> change { plugins.core.setToolAllowAll(id, caller, on) } }
            Tools(current.tools, current.toolsError, enabled) { tool, on ->
                change { plugins.core.setToolReadOnly(id, tool, on) }
            }
        }
    }
}

@Composable
private fun OpenWindows(windows: List<OpenWindow>, enabled: Boolean, onStop: (String) -> Unit) {
    if (windows.isEmpty()) return
    val colors = WontonTheme.colors
    DetailSection("放行中") {
        windows.forEach { window ->
            val until = DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(window.expiresAtMs.toLong()))
            Row(
                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    "${window.name} 放行到 $until",
                    Modifier.weight(1f),
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                )
                WontonButton("停止", { onStop(window.card) }, kind = WontonButtonKind.SECONDARY, enabled = enabled)
            }
        }
    }
}

@Composable
fun BotSwitches(
    title: String,
    risk: String,
    bots: List<BotSwitch>,
    enabled: Boolean,
    onChange: (String, Boolean) -> Unit,
) {
    if (bots.isEmpty()) return
    val colors = WontonTheme.colors
    DetailSection(title) {
        Text(risk, style = WontonTheme.typography.supporting, color = colors.warning.toColor())
        bots.forEach { bot ->
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    bot.name,
                    Modifier.weight(1f),
                    style = WontonTheme.typography.body,
                    color = colors.textPrimary.toColor(),
                )
                WontonSwitch(bot.enabled, { onChange(bot.caller, it) }, enabled)
            }
        }
    }
}

@Composable
private fun Tools(
    tools: List<ToolPolicyEntry>,
    error: String?,
    enabled: Boolean,
    onChange: (String, Boolean) -> Unit,
) {
    if (tools.isEmpty()) {
        if (error != null) DetailSection("工具") { WontonNotice(WontonIcon.Warning, error) }
        return
    }
    val colors = WontonTheme.colors
    var expanded by remember { mutableStateOf(false) }
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
        Row(
            Modifier.fillMaxWidth()
                .heightIn(min = WontonTheme.size.target)
                .clickable(role = Role.Button) { expanded = !expanded },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text("工具", style = WontonTheme.typography.sectionTitle, color = colors.textPrimary.toColor())
            Icon(
                WontonIcon.Forward,
                contentDescription = null,
                Modifier.size(WontonTheme.size.iconSmall).rotate(if (expanded) EXPANDED_DEGREES else 0f),
                tint = colors.textSecondary.toColor(),
            )
        }
        if (expanded) {
            Text(
                "打开的工具算只读，所有 Bot 调它都不再问",
                style = WontonTheme.typography.supporting,
                color = colors.warning.toColor(),
            )
            tools.forEach { ToolRow(it, enabled, onChange) }
        }
    }
}

/** 插件自己标的只读主人改不了：这里只放宽。 */
@Composable
private fun ToolRow(tool: ToolPolicyEntry, enabled: Boolean, onChange: (String, Boolean) -> Unit) {
    val colors = WontonTheme.colors
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(tool.name, style = WontonTheme.typography.body, color = colors.textPrimary.toColor())
            tool.description?.let {
                Text(
                    it,
                    style = WontonTheme.typography.supporting,
                    color = colors.textSecondary.toColor(),
                    maxLines = DESCRIPTION_LINES,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        if (tool.access == ToolAccess.PLUGIN_READ_ONLY) {
            Text("只读", style = WontonTheme.typography.supporting, color = colors.textSecondary.toColor())
        } else {
            WontonSwitch(tool.access == ToolAccess.OWNER_READ_ONLY, { onChange(tool.name, it) }, enabled)
        }
    }
}
