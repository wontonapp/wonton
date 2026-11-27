import Foundation
import UserNotifications

/// 通知服务扩展：App 被杀或锁屏时，推送在这里解开并填好发送者和会话（ADR-0003 的“设计：加密推送”）。
/// 扩展里不跑完整核心，通知里没有正文：解开了显示发送者和会话名，解不开显示一条通用的“有新消息”，
/// 都不丢通知。App 锁着时标题换成 App 名（ADR-0102 §4）。点开后的取消息由主 App 做（和 Android 一致：推送只是唤醒信号）。
final class NotificationService: UNNotificationServiceExtension {
    private var content: UNMutableNotificationContent?
    private var handler: ((UNNotificationContent) -> Void)?

    override func didReceive(
        _ request: UNNotificationRequest, withContentHandler contentHandler: @escaping (UNNotificationContent) -> Void
    ) {
        handler = contentHandler
        content = request.content.mutableCopy() as? UNMutableNotificationContent
        deliver()
    }

    override func serviceExtensionTimeWillExpire() {
        deliver()
    }

    private func deliver() {
        guard let content, let handler else { return }
        self.content = nil
        self.handler = nil
        handler(fill(content))
    }

    /// 解开推送（中继放进 `wp.b` 的 aes128gcm 密文），填好标题和正文；解不开就显示通用的“有新消息”。
    /// 解不开的推送不是家里的服务器发的：只有它知道这台设备的公钥和认证密钥（同 Android 的做法）。
    private func fill(_ content: UNMutableNotificationContent) -> UNNotificationContent {
        let decrypted = pushBody(from: content.userInfo)
            .flatMap { body -> WontonPushPayload? in
                guard let (privateKey, auth) = PushKeys.sharedMaterial() else { return nil }
                return try? WontonPush.decrypt(body, privateKey: privateKey, auth: auth)
            }
        guard let payload = decrypted else {
            content.title = String(localized: .pushFallbackTitle)
            content.body = String(localized: .pushNewMessage)
            return content
        }
        // App 锁着就不露发送者、房间名和卡片标题：正文本来只有“有新消息”和确认卡片的类别文案，标题换成 App 名。
        if PushLock.lockedNow() {
            content.title = String(localized: .pushFallbackTitle)
        } else {
            switch payload {
            case .message(let message):
                // 标题和 Android 一致：群聊用房间名，私聊用发送者。
                content.title = message.roomName
                    ?? message.senderDisplayName ?? message.sender ?? String(localized: .pushUnknownSender)
                content.body = String(localized: .pushNewMessage)
            case .confirmation(let direct):
                // 标题就是卡片标题（核心已按收推送那台设备的语言写好），正文一句类别文案，不重复标题。
                content.title = direct.confirmation.title
                content.body = String(localized: .pushWaitingConfirmation)
            }
        }
        switch payload {
        case .message(let message):
            content.threadIdentifier = message.roomId
            content.userInfo = content.userInfo.merging([
                "server": message.server, "room_id": message.roomId
            ]) { current, _ in current }
        case .confirmation(let direct):
            content.threadIdentifier = direct.confirmation.origin?.roomId ?? direct.confirmation.id
            // 没有来源会话的卡片 room_id 为空，点开后壳落到收件箱。卡片 ID 给 App 撤通知用：卡片结束了
            // （别的设备答了、过期、撤回），App 取到收件箱时按它把这条撤掉（`InboxModel`，键名两边一致）。
            content.userInfo = content.userInfo.merging([
                "server": direct.server, "room_id": direct.confirmation.origin?.roomId ?? "",
                "confirmation_id": direct.confirmation.id
            ]) { current, _ in current }
        }
        return content
    }

    /// 中继封装的约定：APNs 载荷的自定义键 `wp.b` 里是 Web Push 密文（aes128gcm，base64url）。
    private func pushBody(from userInfo: [AnyHashable: Any]) -> Data? {
        guard let webPush = userInfo["wp"] as? [AnyHashable: Any],
            let encoded = webPush["b"] as? String
        else { return nil }
        var base64 = encoded
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        if base64.count % 4 != 0 { base64.append(String(repeating: "=", count: 4 - base64.count % 4)) }
        return Data(base64Encoded: base64)
    }
}
