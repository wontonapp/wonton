package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.Action
import app.wonton.protocol.BotActivity
import app.wonton.protocol.BotRef
import app.wonton.protocol.BotView
import app.wonton.protocol.BotWebFailure
import app.wonton.protocol.BotWebView
import app.wonton.protocol.TabRef
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonDialog
import app.wonton.ui.design.WontonDialogAction
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPadding

@Composable
internal fun BotProfileScreen(
    identity: BotRef, bot: BotView?, plugins: PluginsState, onAction: (Action) -> Unit, onClose: () -> Unit,
) {
    WontonScreen(bar = { WontonPushedBar(stringResource(R.string.bot_profile_title), onClose) }) {
        Column(
            Modifier.fillMaxSize().testTag("bot-profile").verticalScroll(rememberScrollState())
                .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
            verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            DetailSection(stringResource(R.string.bot_identity_title)) {
                Text(
                    stringResource(R.string.bot_identity_value, identity.pluginId, identity.bot),
                    Modifier.testTag("bot-profile-identity"),
                    style = WontonTheme.typography.body,
                    color = WontonTheme.colors.textPrimary.toColor(),
                )
                bot?.let {
                    Text(it.userId, Modifier.testTag("bot-profile-user-id"),
                        style = WontonTheme.typography.supporting,
                        color = WontonTheme.colors.textSecondary.toColor())
                }
            }
            if (bot == null) {
                Text(stringResource(R.string.bot_profile_missing), Modifier.testTag("bot-profile-unavailable"),
                    color = WontonTheme.colors.textSecondary.toColor())
            } else {
                BotProfileSection(bot, plugins)
            }
            BotWebSection(identity, bot?.web, onAction)
        }
    }
}

/** Profile actions address only the default Bot in the existing protocol. */
@Composable
internal fun ReadOnlyBotProfile(bot: BotView, plugins: PluginsState) {
    DetailSection(stringResource(R.string.bot_profile_title)) {
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16)) {
            BotAvatar(bot, plugins, WontonTheme.size.detailIcon)
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                Text(bot.name, style = WontonTheme.typography.itemTitle,
                    color = WontonTheme.colors.textPrimary.toColor())
                Text(stringResource(R.string.bot_profile_read_only), Modifier.testTag("bot-profile-read-only"),
                    style = WontonTheme.typography.supporting,
                    color = WontonTheme.colors.textSecondary.toColor())
            }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s12)) {
            listOf(BotActivity.IDLE, BotActivity.WORKING, BotActivity.WAITING, BotActivity.OFFLINE).forEach { state ->
                Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
                    bot.stateAvatars.firstOrNull { it.state == state }?.image?.let { image ->
                        BotAvatarMedia(image, plugins, WontonTheme.size.previewIcon,
                            fallbackStill = bot.avatar, fallbackIcon = plugins.iconFor(bot.bot.pluginId))
                    }
                    Text(state.label(), style = WontonTheme.typography.caption,
                        color = WontonTheme.colors.textSecondary.toColor())
                }
            }
        }
    }
}

@Composable
private fun BotWebSection(identity: BotRef, web: BotWebView?, onAction: (Action) -> Unit) {
    val ready = (web as? BotWebView.Ready)?.page?.takeIf { it.bot == identity }
    DetailSection(stringResource(R.string.bot_web_title)) {
        if (ready != null) {
            Text(ready.url, Modifier.testTag("bot-web-address"), style = WontonTheme.typography.body,
                color = WontonTheme.colors.textPrimary.toColor())
            WontonButton(
                stringResource(R.string.bot_web_open), { onAction(Action.OpenPage(TabRef.Bot(identity, null))) },
                Modifier.fillMaxWidth().testTag("bot-web-open"),
            )
        } else {
            val reason = (web as? BotWebView.Unavailable)?.reason
            Text(stringResource(reason.webReasonRes()), Modifier.testTag("bot-web-unavailable"),
                style = WontonTheme.typography.body, color = WontonTheme.colors.textSecondary.toColor())
        }
    }
}

private fun BotWebFailure?.webReasonRes(): Int = when (this) {
    BotWebFailure.UNAUTHENTICATED -> R.string.bot_web_unauthenticated
    BotWebFailure.NOT_SHARED -> R.string.bot_web_not_shared
    BotWebFailure.PERMISSION_DENIED -> R.string.bot_web_permission_denied
    BotWebFailure.NOT_FOUND -> R.string.bot_web_not_found
    BotWebFailure.NO_WEBUI -> R.string.bot_web_no_webui
    BotWebFailure.NOT_READY -> R.string.bot_web_not_ready
    BotWebFailure.OTHER, null -> R.string.bot_web_unknown
}

/** Shown by the existing tabs container before a Bot page creates or resumes its WebView. */
@Composable
internal fun BotWebLoginDisclosure(onContinue: () -> Unit, onCancel: () -> Unit) {
    WontonDialog(
        stringResource(R.string.bot_web_login_title),
        stringResource(R.string.bot_web_login_warning),
        onCancel,
        actions = listOf(
            WontonDialogAction(stringResource(R.string.common_cancel), WontonButtonKind.SECONDARY, onCancel),
            WontonDialogAction(stringResource(R.string.bot_web_open), onClick = onContinue),
        ),
        detail = {
            Text(stringResource(R.string.bot_web_shared_state), Modifier.testTag("bot-web-login-disclosure"),
                style = WontonTheme.typography.body, color = WontonTheme.colors.textSecondary.toColor())
        },
    )
}
