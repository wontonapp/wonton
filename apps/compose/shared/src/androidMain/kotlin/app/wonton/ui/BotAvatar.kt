package app.wonton.ui

import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.tween
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import app.wonton.client.botCurrentAvatar
import app.wonton.protocol.AvatarImage
import app.wonton.protocol.AvatarKind
import app.wonton.protocol.BotActivity
import app.wonton.protocol.BotView
import app.wonton.protocol.PluginIcon
import app.wonton.ui.design.generated.WontonMotion

/**
 * Bot 的头像（ADR-0044）：正在的状态头像在播，没有状态头像显示静态头像，都没有用应用的图标。
 * 兜底顺序由核心的 [botCurrentAvatar] 算，壳不自己挑。
 *
 * 省电规则是硬性的（[avatarAllowed]、[rememberPlayingSlot]）：只在 App 在前台、头像在屏幕上时播；
 * 一屏最多播 4 个，其余显示首帧；省电模式或减弱动态效果时只显示首帧。视频静音循环，不请求音频焦点。
 */
@Composable
fun BotAvatar(bot: BotView, plugins: PluginsState, size: Dp, modifier: Modifier = Modifier) {
    // 状态变化时新旧头像淡入淡出（约 200 ms；减弱动态效果时不超过 100 ms）。
    // 兜底顺序在核心里算；bot 变了才算一次，滚动重排不再过 FFI。
    val current = remember(bot) { botCurrentAvatar(bot) }
    val label = avatarLabel(bot)
    Crossfade(
        targetState = current ?: bot.avatar,
        animationSpec = avatarFade(),
        modifier = modifier.semantics { contentDescription = label },
        label = "botAvatar",
    ) { image ->
        if (image == null) {
            PluginIconImage(plugins.iconFor(bot.bot.pluginId), size, plugins)
        } else {
            BotAvatarMedia(
                image,
                plugins,
                size,
                fallbackStill = bot.avatar.takeIf { it != image },
                fallbackIcon = plugins.iconFor(bot.bot.pluginId),
            )
        }
    }
}

/** 详情页一格状态头像：固定显示 [image] 这一个，同样遵守省电规则；[image] 为空由调用方画空格子。 */
@Composable
fun BotAvatarMedia(
    image: AvatarImage,
    plugins: PluginsState,
    size: Dp,
    fallbackStill: AvatarImage?,
    fallbackIcon: PluginIcon?,
    modifier: Modifier = Modifier,
) {
    val allowed = avatarAllowed()
    Box(modifier.size(size), contentAlignment = Alignment.Center) {
        when (image.kind) {
            AvatarKind.STILL -> StillAvatar(image, plugins, size, fallbackStill, fallbackIcon)
            AvatarKind.ANIMATED -> AnimatedAvatar(image, allowed, plugins, size, fallbackStill, fallbackIcon)
            AvatarKind.VIDEO -> VideoAvatar(image, allowed, plugins, size, fallbackStill, fallbackIcon)
        }
    }
}

@Composable
private fun avatarLabel(bot: BotView): String = when (val activity = bot.activity) {
    null -> bot.name
    else -> stringResource(R.string.bot_avatar_label, bot.name, activity.label())
}

/** 状态的叫法（规格第 4 节）：待机 / 工作中 / 等你确认 / 离线。 */
@Composable
fun BotActivity.label(): String = stringResource(labelRes())

/** [label] 的资源；不在 Compose 里（桌面小组件）时用它取字。 */
fun BotActivity.labelRes(): Int = when (this) {
    BotActivity.IDLE -> R.string.bot_state_idle
    BotActivity.WORKING -> R.string.bot_state_working
    BotActivity.WAITING -> R.string.bot_state_waiting
    BotActivity.OFFLINE -> R.string.bot_state_offline
}

private fun avatarFade() = tween<Float>(
    if (AvatarEnvironment.reduceMotion) WontonMotion.reduced else WontonMotion.state,
    easing = LinearEasing,
)
