package app.wonton.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.union
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
import app.wonton.protocol.CoreException
import app.wonton.protocol.OwnerProfile
import app.wonton.protocol.OwnerProfileUpdate
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
    val loaded by produceState<Result<OwnerProfile>?>(null, attempt) {
        value = null
        value = try {
            val profile = plugins.core.ownerProfile()
            name = profile.displayName.orEmpty()
            email = profile.email.orEmpty()
            Result.success(profile)
        } catch (failure: CoreException) {
            Result.failure(failure)
        }
    }
    WontonScreen(bar = { WontonPushedBar("你的资料", onClose, WontonIcon.Close, "关闭") }) {
        val result = loaded
        val profile = result?.getOrNull()
        // 保存按钮顶在键盘上面，不然填完要先收起键盘才点得到。
        Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.ime.union(WindowInsets.navigationBars))) {
            Box(Modifier.weight(1f).fillMaxWidth()) {
                when {
                    result == null -> LoadingView()
                    result.isFailure -> ProblemView(result.failureProblem, onRetry = { attempt++ })
                    profile != null -> OwnerProfileForm(
                        profile, name, { name = it }, email, { email = it }, problem,
                        Modifier.fillMaxSize().verticalScroll(rememberScrollState())
                            .padding(WontonTheme.spacing.pageInset),
                    )
                }
            }
            if (profile != null) {
                WontonButton(
                    "保存",
                    {
                        problem = null
                        saving = true
                        scope.launch {
                            try {
                                plugins.core.saveOwnerProfile(OwnerProfileUpdate(name, email))
                                onClose()
                            } catch (failure: CoreException) {
                                problem = failure.problem
                            }
                            saving = false
                        }
                    },
                    Modifier.padding(WontonTheme.spacing.pageInset).fillMaxWidth(),
                    enabled = !saving && name.isNotBlank(),
                )
            }
        }
    }
}
