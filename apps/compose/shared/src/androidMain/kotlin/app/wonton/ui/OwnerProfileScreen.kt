package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import app.wonton.protocol.CoreException
import app.wonton.protocol.Me
import app.wonton.protocol.OwnerProfileUpdate
import app.wonton.ui.design.wontonContentPadding
import app.wonton.ui.design.LocalWontonBottomInset
import app.wonton.ui.design.WontonButton
import app.wonton.ui.design.WontonPushedBar
import app.wonton.ui.design.WontonScreen
import app.wonton.ui.design.WontonTheme
import app.wonton.ui.icons.Close
import app.wonton.ui.icons.WontonIcon
import kotlinx.coroutines.launch

/** 你的资料（ADR-0030）：名字和邮箱存在家里的服务器上，要身份的应用从那里取。规则在家里的服务器上，不合规则的原因原样显示。 */
@Composable
fun OwnerProfilePage(plugins: PluginsState, onClose: () -> Unit) {
    BackHandler(onBack = onClose)
    var attempt by remember { mutableIntStateOf(0) }
    var name by remember { mutableStateOf("") }
    var email by remember { mutableStateOf("") }
    var problem by remember { mutableStateOf<Problem?>(null) }
    var saving by remember { mutableStateOf(false) }
    val scope = rememberCoroutineScope()
    val loaded by produceState<Result<Me>?>(null, attempt) {
        value = null
        value = try {
            val profile = plugins.core.me()
            name = profile.displayName.orEmpty()
            email = profile.email.orEmpty()
            Result.success(profile)
        } catch (failure: CoreException) {
            Result.failure(failure)
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
                        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).wontonContentPadding()
                            .padding(WontonTheme.spacing.pageInset),
                    )
                }
            }
            if (profile != null) {
                WontonButton(
                    stringResource(R.string.common_save),
                    {
                        problem = null
                        saving = true
                        scope.launch {
                            try {
                                plugins.core.saveMe(OwnerProfileUpdate(name, email))
                                onClose()
                            } catch (failure: CoreException) {
                                problem = failure.problem
                            }
                            saving = false
                        }
                    },
                    Modifier.padding(bottom = LocalWontonBottomInset.current)
                        .padding(WontonTheme.spacing.pageInset).fillMaxWidth(),
                    enabled = !saving && name.isNotBlank(),
                )
            }
        }
    }
}
