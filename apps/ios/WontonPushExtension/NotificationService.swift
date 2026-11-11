import Foundation
import UserNotifications

/// 通知服务扩展：App 被杀或锁屏时，推送在这里解开并填好发送者和会话（ADR-0003 的“设计：加密推送”）。
/// 扩展里不跑完整核心，通知里没有正文：解开了显示发送者和会话名，解不开显示一条通用的“有新消息”，
/// 都不丢通知。点开后的取消息由主 App 做（和 Android 一致：推送只是唤醒信号）。
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
        // 标题和 Android 一致：群聊用房间名，私聊用发送者（推送里没有正文可显示）。
        content.title = payload.roomName
            ?? payload.senderDisplayName ?? payload.sender ?? String(localized: .pushUnknownSender)
        content.body = String(localized: .pushNewMessage)
        content.threadIdentifier = payload.roomId
        // 点开通知的跳转用：通知来自哪台服务器、进哪个房间。
        content.userInfo = content.userInfo.merging([
            "server": payload.server, "room_id": payload.roomId,
        ]) { current, _ in current }
        return content
    }

    /// 中继封装的约定：APNs 载荷的自定义键 `wp.b` 里是 Web Push 密文（aes128gcm，base64url）。
    private func pushBody(from userInfo: [AnyHashable: Any]) -> Data? {
        guard let wp = userInfo["wp"] as? [AnyHashable: Any],
            let encoded = wp["b"] as? String
        else { return nil }
        var base64 = encoded
            .replacingOccurrences(of: "-", with: "+")
            .replacingOccurrences(of: "_", with: "/")
        if base64.count % 4 != 0 { base64.append(String(repeating: "=", count: 4 - base64.count % 4)) }
        return Data(base64Encoded: base64)
    }
}
