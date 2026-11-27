package app.wonton.ui

import app.wonton.client.ClientCore
import app.wonton.protocol.FamilyViewModel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

/** 把成员页的订阅包装成 [Flow]，取消和释放方式同 [chatUpdates]。 */
fun ClientCore.familyUpdates(): Flow<FamilyViewModel> = flow {
    subscribeFamily().use { subscription ->
        while (true) {
            emit(subscription.next() ?: break)
        }
    }
}

/// 预览不带原生核心时也要判断权限，按字段直接算，不调 UniFFI 生成的方法。
fun app.wonton.protocol.FamilyAccess.canManageRoles() = isOwner

fun app.wonton.protocol.FamilyAccess.canManageMembers() = isOwner || permissions?.manageMembers == true
