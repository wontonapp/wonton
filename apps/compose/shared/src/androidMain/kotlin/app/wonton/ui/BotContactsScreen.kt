package app.wonton.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import app.wonton.client.ClientCore
import app.wonton.protocol.Action
import app.wonton.protocol.BotContact
import app.wonton.protocol.BotContactsState
import app.wonton.protocol.BotContactsViewModel
import app.wonton.protocol.BotRef
import app.wonton.protocol.ContactState
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonButtonKind
import app.wonton.ui.design.WontonGroup
import app.wonton.ui.design.WontonIconButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonRow
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonSeparator
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.icons.Add
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/** 把 Bot 联系人的订阅包装成 [Flow]，取消和释放方式同其他核心订阅。 */
fun ClientCore.botContactsUpdates(): Flow<BotContactsViewModel> = flow {
    subscribeBotContacts().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/** Bot 详情页“资料”下面去联系人页的一行（ADR-0045、ADR-0050）。 */
@Composable
internal fun BotContactsEntry(onOpen: () -> Unit) {
    WontonGroup {
        WontonRow(stringResource(R.string.contacts_title), showsForward = true, onClick = onOpen)
    }
}

/**
 * 一个 Bot 的联系人（ADR-0045、ADR-0050）：同机和跨机是同一份列表，状态、失败原因都来自核心的
 * [ClientCore.botContactsUpdates]。进页面选定这个 Bot 取一次；之后的“等对方”由核心自己跟。
 */
@Composable
internal fun BotContactsPage(core: ClientCore, bot: BotRef, onBack: () -> Unit) {
    val view by remember(core) { core.botContactsUpdates() }.collectAsState(remember(core) { core.botContacts() })
    var adding by remember { mutableStateOf(false) }
    var removing by remember { mutableStateOf<BotContact?>(null) }
    var rejection by remember { mutableStateOf<Problem?>(null) }
    val send = { action: Action -> rejection = core.send(action) }
    LaunchedEffect(core, bot) { send(Action.RefreshBotContacts(bot)) }
    WontonScreen(bar = {
        WontonPushedBar(stringResource(R.string.contacts_title), onBack, actions = {
            WontonIconButton(WontonIcon.Add, stringResource(R.string.contacts_add), { adding = true })
        })
    }) {
        BotContactsContent(view, bot, send, onAdd = { adding = true }, onRemove = { removing = it })
    }
    if (adding) {
        AddContactSheet(onDismiss = { adding = false }) { userId, person ->
            send(
                if (person) Action.RequestPersonContact(bot, userId) else Action.RequestBotContact(bot, userId),
            )
        }
    }
    removing?.let { contact ->
        RemoveContactDialog(contact, onDismiss = { removing = null }) {
            removing = null
            send(Action.RemoveBotContact(bot, contact.userId))
        }
    }
    rejection?.let { ProblemDialog(it) { rejection = null } }
}

/**
 * 联系人页的内容：列表、整页的失败、转圈或空页。ViewModel 还看着别的 Bot 时（刚切过来）当成正在取。
 * 删掉已生效的联系人要确认，经 [onRemove] 交给页面弹框；其余操作直接发给核心。
 */
@Composable
fun BotContactsContent(
    view: BotContactsViewModel,
    bot: BotRef,
    onAction: (Action) -> Unit,
    onAdd: () -> Unit,
    onRemove: (BotContact) -> Unit,
) {
    val current = view.bot == bot
    val contacts = if (current) view.contacts else emptyList()
    val failed = (view.state as? BotContactsState.Failed)?.takeIf { current }
    val retry = { onAction(Action.RefreshBotContacts(bot)) }
    when {
        contacts.isNotEmpty() -> BotContactList(contacts, failed, retry, onAction, bot, onRemove)
        failed != null -> ProblemView(failed.problem(), retry)
        !current || view.state == BotContactsState.Loading -> LoadingView()
        else -> BotContactsEmpty(onAdd)
    }
}

@Composable
private fun BotContactList(
    contacts: List<BotContact>,
    failed: BotContactsState.Failed?,
    onRetry: () -> Unit,
    onAction: (Action) -> Unit,
    bot: BotRef,
    onRemove: (BotContact) -> Unit,
) {
    val incoming = contacts.filter { it.state == ContactState.INCOMING }
    val active = contacts.filter { it.state == ContactState.ACTIVE }
    val sent = contacts.filter { it.state != ContactState.INCOMING && it.state != ContactState.ACTIVE }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
    ) {
        failed?.let { ProblemRow(it.problem(), onRetry) }
        ContactSection(R.string.contacts_requests_title, incoming) { contact ->
            IncomingContactRow(
                contact,
                onApprove = { onAction(Action.ApproveBotContact(bot, contact.userId)) },
                onReject = { onAction(Action.RejectBotContact(bot, contact.userId)) },
            )
        }
        ContactSection(R.string.contacts_active_title, active) { contact ->
            ActiveContactRow(contact) { onRemove(contact) }
        }
        ContactSection(R.string.contacts_sent_title, sent) { contact ->
            // 撤销请求、清掉被拒或查无此人的行都不用确认：对方那边什么都没成立。
            SentContactRow(contact) { onAction(Action.RemoveBotContact(bot, contact.userId)) }
        }
    }
}

/** 一节：小标题在组上面；没有行时整节不占位置。 */
@Composable
private fun ContactSection(title: Int, rows: List<BotContact>, row: @Composable (BotContact) -> Unit) {
    if (rows.isEmpty()) return
    Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s4)) {
        Text(
            stringResource(title),
            style = WontonTheme.typography.sectionTitle,
            color = WontonTheme.colors.textPrimary.toColor(),
        )
        WontonGroup {
            rows.forEachIndexed { index, contact ->
                if (index > 0) WontonSeparator()
                row(contact)
            }
        }
    }
}

/** 还没有联系人：一句话和添加。 */
@Composable
private fun BotContactsEmpty(onAdd: () -> Unit) {
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
            .wontonContentPadding().padding(WontonTheme.spacing.pageInset),
        verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s16, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            stringResource(R.string.contacts_empty),
            style = WontonTheme.typography.body.copy(textAlign = TextAlign.Center),
            color = WontonTheme.colors.textSecondary.toColor(),
        )
        WontonButton(stringResource(R.string.contacts_add), onAdd, kind = WontonButtonKind.SECONDARY)
    }
}
