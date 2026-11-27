package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.JoinAbilityState
import app.wonton.protocol.JoinRoleView
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.CheckCircle
import app.wonton.ui.icons.Clock
import app.wonton.ui.icons.Lock
import app.wonton.ui.icons.WontonIcon

/** 被邀请人的加入页（ADR-0088）：说清给什么角色、能做什么，再决定加入还是算了。 */
@Composable
fun JoinInviteScreen(role: JoinRoleView?, onAccept: (() -> Unit)?, onDecline: () -> Unit) {
    Column(
        Modifier.verticalScroll(rememberScrollState()).padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16),
    ) {
        Text(stringResource(R.string.family_joinTitle), style = WontonTheme.typography.itemTitle)
        Text(
            stringResource(R.string.family_joinBlurb),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        role?.let {
            val name = it.name
            WontonGroup {
                Column(
                    Modifier.padding(WontonTheme.spacing.s16),
                    verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                ) {
                    if (!name.isNullOrEmpty()) {
                        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                            Text(name, style = WontonTheme.typography.itemTitle)
                            it.template?.let { template ->
                                Text(templateName(template), style = WontonTheme.typography.caption,
                                    color = WontonTheme.colors.textSecondary.toColor())
                            }
                        }
                    } else {
                        Text(
                            it.template?.let { template -> templateName(template) } ?: it.roleId,
                            style = WontonTheme.typography.itemTitle,
                        )
                    }
                    if (it.abilities.isNotEmpty()) {
                        Text(stringResource(R.string.family_joinAbilities),
                            style = WontonTheme.typography.supporting,
                            color = WontonTheme.colors.textSecondary.toColor())
                        it.abilities.forEach { ability ->
                            Row(
                                horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8),
                                verticalAlignment = Alignment.Top,
                            ) {
                                Icon(
                                    abilityIcon(ability.state),
                                    contentDescription = null,
                                    modifier = Modifier.size(WontonTheme.size.iconSmall),
                                    tint = WontonTheme.colors.textSecondary.toColor(),
                                )
                                Text(abilityText(ability), style = WontonTheme.typography.body)
                            }
                        }
                    }
                }
            }
        }
        Text(
            stringResource(R.string.family_joinPrivacy),
            style = WontonTheme.typography.supporting,
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
            if (onAccept != null) {
                WontonButton(stringResource(R.string.family_joinAccept), onAccept)
            }
            WontonButton(stringResource(R.string.family_joinDecline), onDecline, kind = WontonButtonKind.SECONDARY)
        }
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.Center,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            CircularProgressIndicator(Modifier.size(WontonTheme.size.iconSmall))
            Text(
                "  " + stringResource(R.string.family_joinWaiting),
                style = WontonTheme.typography.supporting,
                color = WontonTheme.colors.textSecondary.toColor(),
            )
        }
    }
}

private fun abilityIcon(state: JoinAbilityState): ImageVector = when (state) {
    JoinAbilityState.ALLOWED -> WontonIcon.CheckCircle
    JoinAbilityState.NEEDS_ADMIN -> WontonIcon.Clock
    JoinAbilityState.NOT_ALLOWED, JoinAbilityState.OTHER -> WontonIcon.Lock
}
