package app.wonton.ui

import android.net.Uri
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts.PickVisualMedia
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.CoreException
import app.wonton.protocol.Me
import app.wonton.protocol.MyAvatarProblem
import app.wonton.protocol.OwnerProfileUpdate
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonSmallButton
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.design.toColor
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.WontonIcon
import java.io.IOException
import kotlinx.coroutines.launch

/** 你的资料（ADR-0030）：名字和邮箱存在家里的服务器上，要身份的应用从那里取。规则在家里的服务器上，不合规则的原因原样显示。 */
@Composable
fun OwnerProfilePage(plugins: PluginsState, onClose: () -> Unit) {
    BackHandler(onBack = onClose)
    var attempt by remember { mutableIntStateOf(0) }
    var name by remember { mutableStateOf("") }
    var email by remember { mutableStateOf("") }
    var problem by remember { mutableStateOf<Problem?>(null) }
    var avatarProblem by remember { mutableStateOf<Problem?>(null) }
    var avatarBusy by remember { mutableStateOf(false) }
    var loaded by remember { mutableStateOf<Result<Me>?>(null) }
    val scope = rememberCoroutineScope()
    LaunchedEffect(attempt) {
        loaded = null
        loaded = try {
            val profile = plugins.core.me()
            name = profile.displayName.orEmpty()
            email = profile.email.orEmpty()
            Result.success(profile)
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
    val context = LocalContext.current
    val unreadableText = stringResource(R.string.bot_media_other)
    // 资料头像只收静态图：动图、视频都在核心那儿不过。
    val pickAvatar = rememberLauncherForActivityResult(PickVisualMedia()) { uri: Uri? ->
        if (uri == null) return@rememberLauncherForActivityResult
        scope.launch {
            avatarBusy = true
            avatarProblem = null
            try {
                val picked = AvatarTranscode.make(context, uri, plugins.core, forState = false, stillOnly = true) {}
                if (picked is AvatarPick.Ready) loaded = Result.success(plugins.core.setMyAvatar(picked.upload))
            } catch (failure: MyAvatarProblem) {
                avatarProblem = failure.problem
            } catch (failure: IOException) {
                avatarProblem = Problem(unreadableText, detail = failure.message)
            } finally {
                avatarBusy = false
            }
        }
    }
    val clearAvatar: () -> Unit = {
        scope.launch {
            avatarBusy = true
            avatarProblem = null
            try {
                loaded = Result.success(plugins.core.clearMyAvatar())
            } catch (failure: MyAvatarProblem) {
                avatarProblem = failure.problem
            } finally {
                avatarBusy = false
            }
        }
    }
    WontonScreen(
        bar = {
            WontonPushedBar(
                stringResource(R.string.owner_profile_title), onClose, WontonIcon.Close,
                stringResource(R.string.design_close),
            )
        },
    ) {
        val result = loaded
        val profile = result?.getOrNull()
        // 保存按钮顶在键盘上面，不然填完要先收起键盘才点得到。
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.ime)) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    result == null -> LoadingView()
                    result.isFailure -> ProblemView(result.failureProblem, onRetry = { attempt++ })
                    profile != null -> OwnerProfileForm(
                        profile, name, { name = it }, email, { email = it }, problem,
                        avatar = {
                            OwnerAvatarSection(profile, plugins, avatarBusy, avatarProblem, clearAvatar) {
                                pickAvatar.launch(PickVisualMediaRequest(PickVisualMedia.ImageOnly))
                            }
                        },
                        modifier = Modifier.fillMaxSize().verticalScroll(rememberScrollState()).wontonContentPadding()
                            .padding(WontonTheme.spacing.pageInset),
                    )
                }
            }
            if (profile != null) {
                OwnerSaveBar(plugins, name, email, { problem = it }, onClose)
            }
        }
    }
}

/** 保存按钮顶在键盘上面，没填名字时灰掉。 */
@Composable
private fun OwnerSaveBar(
    plugins: PluginsState,
    name: String,
    email: String,
    onProblem: (Problem?) -> Unit,
    onClose: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var saving by remember { mutableStateOf(false) }
    WontonButton(
        stringResource(R.string.common_save),
        {
            onProblem(null)
            saving = true
            scope.launch {
                try {
                    plugins.core.saveMe(OwnerProfileUpdate(name, email))
                    onClose()
                } catch (failure: CoreException) {
                    onProblem(failure.problem)
                }
                saving = false
            }
        },
        Modifier.padding(bottom = LocalWontonBottomInset.current)
            .padding(WontonTheme.spacing.pageInset).fillMaxWidth(),
        enabled = !saving && name.isNotBlank(),
    )
}

/** 自己的头像：圆形当前图（没设时名字首字母），旁边选图和移除；失败只说类别，重试就是再选一次。 */
@Composable
private fun OwnerAvatarSection(
    profile: Me,
    plugins: PluginsState,
    busy: Boolean,
    problem: Problem?,
    onClear: () -> Unit,
    onPick: () -> Unit,
) {
    DetailSection(stringResource(R.string.owner_profile_avatar)) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(WontonTheme.spacing.sectionGap),
        ) {
            Box(Modifier.size(WontonTheme.size.detailIcon).clip(CircleShape), contentAlignment = Alignment.Center) {
                val avatar = profile.avatar
                if (avatar != null) {
                    StillAvatar(
                        avatar, plugins, WontonTheme.size.detailIcon,
                        fallbackStill = null, fallbackIcon = null,
                    )
                } else {
                    Box(
                        Modifier.fillMaxSize().background(WontonTheme.colors.surfaceSubtle.toColor()),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            (profile.displayName?.trim()?.takeIf { it.isNotEmpty() } ?: profile.username).take(1),
                            style = WontonTheme.typography.screenTitle,
                            color = WontonTheme.colors.textSecondary.toColor(),
                        )
                    }
                }
                if (busy) CircularProgressIndicator(Modifier.size(WontonTheme.spacing.s24))
            }
            Column(verticalArrangement = Arrangement.spacedBy(WontonTheme.spacing.s8)) {
                WontonSmallButton(stringResource(R.string.owner_profile_avatar_pick), onPick, enabled = !busy)
                if (profile.avatar != null) {
                    WontonSmallButton(
                        stringResource(R.string.owner_profile_avatar_remove), onClear,
                        enabled = !busy, danger = true,
                    )
                }
            }
        }
        problem?.let { ProblemNotice(it) }
    }
}

