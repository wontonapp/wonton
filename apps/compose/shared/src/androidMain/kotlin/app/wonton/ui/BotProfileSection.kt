package app.wonton.ui

import android.content.Context
import android.net.Uri
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardCapitalization
import app.wonton.protocol.Action
import app.wonton.protocol.AvatarMediaProblem
import app.wonton.protocol.BotActivity
import app.wonton.protocol.BotNameProblem
import app.wonton.protocol.BotProfileFailure
import app.wonton.protocol.BotProfileProblem
import app.wonton.protocol.BotView
import app.wonton.protocol.PluginIcon
import app.wonton.ui.design.WontonActionSheet
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonNotice
import app.wonton.ui.design.WontonSheetAction
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonTextField
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.Forward
import app.wonton.ui.icons.Image
import app.wonton.ui.icons.Trash
import app.wonton.ui.icons.Warning
import app.wonton.ui.icons.WontonIcon
import java.io.IOException
import kotlinx.coroutines.launch

/** 这个 Bot 的应用的图标（兜底用）；Bot 资料里没设头像时显示它。 */
internal fun PluginsState.iconFor(pluginId: String): PluginIcon? =
    viewModel.installed?.plugins?.firstOrNull { it.id == pluginId }?.icon

/**
 * 已装插件的详情页最上面的一节“资料”（ADR-0044，规格第 2 节）：大头像和名字、静态头像的选图和移除、
 * 4 个状态头像。保存和失败都交给核心：进展在 [BotView.saving]，失败在 [BotView.problem]，壳只显示和关问题。
 */
@Composable
internal fun BotProfileSection(bot: BotView, plugins: PluginsState) {
    if (bot.bot.bot != "default") {
        ReadOnlyBotProfile(bot, plugins)
        return
    }
    DetailSection(stringResource(R.string.bot_profile_title)) {
        var editingName by remember { mutableStateOf(false) }
        var nameDraft by remember { mutableStateOf(bot.name) }
        var localProblem by remember { mutableStateOf<Problem?>(null) }
        var transcoding by remember { mutableStateOf(false) }
        var progress by remember { mutableFloatStateOf(0f) }
        // 面板显隐和“这次选取给哪个状态”分开：面板收起时先跑 onDismiss 再跑选中的操作，
        // pickingState 要活到 Photo Picker 回调，不能跟着面板一起清掉。
        var sheetState by remember { mutableStateOf<BotActivity?>(null) }
        var pickingState by remember { mutableStateOf<BotActivity?>(null) }
        val busy = bot.saving || transcoding

        // 失败的原因核心记进 BotView.problem，由下面的提示条显示，这里不弹窗。
        val send: (Action) -> Unit = { action -> runCatching { plugins.core.dispatch(action) } }
        val context = LocalContext.current
        val scope = rememberCoroutineScope()
        val useVideoText = stringResource(R.string.bot_media_use_video)
        val unreadableText = stringResource(R.string.bot_media_other)
        fun pickAndSend(uri: Uri, state: BotActivity?) {
            scope.launch {
                transcoding = true
                progress = 0f
                localProblem = null
                val picked = runCatching {
                    AvatarTranscode.make(context, uri, plugins.core, forState = state != null) { progress = it }
                }
                transcoding = false
                picked.fold(
                    onSuccess = { result ->
                        when (result) {
                            is AvatarPick.Ready -> send(Action.SetBotAvatar(bot.bot.pluginId, state, result.upload))
                            AvatarPick.UseVideo -> localProblem = Problem(useVideoText)
                        }
                    },
                    onFailure = { failure ->
                        localProblem = Problem(unreadableText, detail = (failure as? IOException)?.message)
                    },
                )
            }
        }
        val pickStatic = rememberLauncherForActivityResult(PickVisualMedia()) { uri ->
            if (uri != null) pickAndSend(uri, null)
        }
        val pickState = rememberLauncherForActivityResult(PickVisualMedia()) { uri ->
            val state = pickingState
            pickingState = null
            if (uri != null && state != null) pickAndSend(uri, state)
        }
        LaunchedEffect(bot.name) { if (!editingName) nameDraft = bot.name }

        ProfileHeader(bot, plugins, busy, editingName, nameDraft, { nameDraft = it },
            onEditName = { editingName = true },
            onSaveName = {
                send(Action.SetBotName(bot.bot.pluginId, nameDraft))
                editingName = false
            },
            onCancelName = { editingName = false },
            onPickStatic = { pickStatic.launch(PickVisualMediaRequest(PickVisualMedia.ImageOnly)) },
            onRemoveStatic = { send(Action.ClearBotAvatar(bot.bot.pluginId, null)) },
        )
        StateGrid(bot, plugins, busy) { state ->
            pickingState = state
            if (bot.stateAvatars.any { it.state == state }) {
                sheetState = state
            } else {
                pickState.launch(PickVisualMediaRequest(PickVisualMedia.ImageAndVideo))
            }
        }
        if (transcoding) LinearProgressIndicator(progress = { progress }, Modifier.fillMaxWidth())
        localProblem?.let { problem -> ProfileProblemNotice(problem) { localProblem = null } }
        bot.problem?.let { problem ->
            ProfileProblemNotice(problem.profileProblem()) { send(Action.DismissBotProfileProblem(bot.bot.pluginId)) }
        }
        sheetState?.let { state ->
            WontonActionSheet(
                listOf(
                    WontonSheetAction(stringResource(R.string.bot_avatar_choose_media), WontonIcon.Image) {
                        pickingState = state
                        pickState.launch(PickVisualMediaRequest(PickVisualMedia.ImageAndVideo))
                    },
                    WontonSheetAction(stringResource(R.string.bot_avatar_remove), WontonIcon.Trash, isDanger = true) {
                        send(Action.ClearBotAvatar(bot.bot.pluginId, state))
                    },
                ),
                onDismiss = { sheetState = null },
                title = state.label(),
            )
        }
    }
}

/** 大头像、名字（点按编辑）和静态头像的选图、移除。 */
@Composable
private fun ProfileHeader(
    bot: BotView,
    plugins: PluginsState,
    busy: Boolean,
    editingName: Boolean,
    nameDraft: String,
    onNameDraft: (String) -> Unit,
    onEditName: () -> Unit,
    onSaveName: () -> Unit,
    onCancelName: () -> Unit,
    onPickStatic: () -> Unit,
    onRemoveStatic: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BotAvatar(bot, plugins, WontonTheme.size.detailIcon)
        Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            if (editingName) {
                WontonTextField(
                    nameDraft,
                    onNameDraft,
                    Modifier.fillMaxWidth(),
                    label = stringResource(R.string.bot_name_title),
                    enabled = !busy,
                    keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                )
                Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                    WontonSmallButton(stringResource(R.string.common_save), onSaveName, enabled = !busy)
                    WontonSmallButton(stringResource(R.string.common_cancel), onCancelName)
                }
            } else {
                NameLine(bot, busy, onEditName)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                WontonSmallButton(stringResource(R.string.bot_avatar_choose), onPickStatic, enabled = !busy)
                WontonSmallButton(
                    stringResource(R.string.bot_avatar_remove),
                    onRemoveStatic,
                    enabled = !busy && bot.avatar != null,
                )
            }
        }
    }
}

/** 名字点进去编辑；保存时核心会 trim、查规则，不行的原因进 [BotView.problem]。 */
@Composable
private fun NameLine(bot: BotView, busy: Boolean, onEditName: () -> Unit) {
    Row(
        Modifier.clickable(enabled = !busy, role = Role.Button, onClick = onEditName),
        horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(bot.name, style = WontonTheme.typography.itemTitle, color = WontonTheme.colors.textPrimary.toColor())
        Icon(
            WontonIcon.Forward,
            // 装饰：整行可点，名字就在左边的文字里。
            contentDescription = null,
            modifier = Modifier.size(WontonTheme.size.iconTiny),
            tint = WontonTheme.colors.textSecondary.toColor(),
        )
        if (bot.saving) CircularProgressIndicator(Modifier.size(WontonTheme.size.iconTiny))
    }
}

/** 4 个状态头像一行四格（规格第 2 节）：没设是空格子加“添加”，设了点开换或移除。 */
@Composable
private fun StateGrid(bot: BotView, plugins: PluginsState, busy: Boolean, onPick: (BotActivity) -> Unit) {
    Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
        listOf(BotActivity.IDLE, BotActivity.WORKING, BotActivity.WAITING, BotActivity.OFFLINE).forEach { state ->
            StateCell(state, bot, plugins, busy) { onPick(state) }
        }
    }
}

@Composable
private fun StateCell(state: BotActivity, bot: BotView, plugins: PluginsState, busy: Boolean, onPick: () -> Unit) {
    val size = WontonTheme.size.previewIcon
    val image = bot.stateAvatars.firstOrNull { it.state == state }?.image
    // 状态名字在格子下面，点起来换图的是这一格本身，读屏要在可点区域上听到换的是哪个状态。
    val pickLabel = stringResource(R.string.bot_avatar_pick_state, state.label())
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4),
    ) {
        if (image == null) {
            Column(
                Modifier.size(size)
                    .clip(CircleShape)
                    .background(WontonTheme.colors.surfaceSubtle.toColor(), CircleShape)
                    .clickable(enabled = !busy, role = Role.Button, onClick = onPick),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Text(
                    stringResource(R.string.bot_avatar_add),
                    style = WontonTheme.typography.caption,
                    color = WontonTheme.colors.textSecondary.toColor(),
                )
            }
        } else {
            BotAvatarMedia(
                image,
                plugins,
                size,
                fallbackStill = bot.avatar,
                fallbackIcon = plugins.iconFor(bot.bot.pluginId),
                modifier = Modifier.clickable(enabled = !busy, role = Role.Button, onClick = onPick)
                    .semantics { contentDescription = pickLabel },
            )
        }
        Text(state.label(), style = WontonTheme.typography.caption, color = WontonTheme.colors.textSecondary.toColor())
    }
}

/** 改资料失败的一句人话（规格第 2 节）：类别由核心给，文字按类别写在壳里；detail 收进默认收起的“详情”。 */
@Composable
private fun BotProfileProblem.profileProblem(): Problem = Problem(
    when (val reason = reason) {
        BotProfileFailure.TransportUnavailable -> stringResource(R.string.bot_problem_transport)
        BotProfileFailure.NotReady -> stringResource(R.string.bot_problem_not_ready)
        is BotProfileFailure.InvalidName -> stringResource(reason.problem.textRes())
        is BotProfileFailure.MediaRejected -> stringResource(reason.problem.textRes())
        BotProfileFailure.Upstream -> stringResource(R.string.bot_problem_upstream)
        BotProfileFailure.PermissionRequired -> stringResource(R.string.bot_problem_owner_only)
        else -> stringResource(R.string.bot_problem_other)
    },
    detail = detail,
)

private fun BotNameProblem.textRes() = when (this) {
    BotNameProblem.EMPTY -> R.string.bot_name_empty
    BotNameProblem.TOO_LONG -> R.string.bot_name_too_long
    BotNameProblem.CONTROL_CHARACTERS -> R.string.bot_name_control
    BotNameProblem.RESERVED -> R.string.bot_name_reserved
    BotNameProblem.OTHER -> R.string.bot_name_other
}

private fun AvatarMediaProblem.textRes() = when (this) {
    AvatarMediaProblem.FORMAT -> R.string.bot_media_format
    AvatarMediaProblem.TOO_LARGE -> R.string.bot_media_too_large
    AvatarMediaProblem.TOO_BIG -> R.string.bot_media_too_big
    AvatarMediaProblem.NOT_SQUARE -> R.string.bot_media_not_square
    AvatarMediaProblem.TOO_LONG -> R.string.bot_media_too_long
    AvatarMediaProblem.UNREADABLE -> R.string.bot_media_unreadable
    AvatarMediaProblem.OTHER -> R.string.bot_media_other
}

/** 资料保存失败的提示条：一句人话加收起的详情，能关掉（照 ProblemNotice 的写法）。 */
@Composable
private fun ProfileProblemNotice(problem: Problem, onClose: () -> Unit) {
    WontonNotice(WontonIcon.Warning, problem.title) {
        problem.detail?.let { ProblemDetail(it) }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
            WontonIconButton(WontonIcon.Close, stringResource(R.string.common_close), onClose)
        }
    }
}
