import CryptoKit
import Foundation
import Security

/// 这台设备收推送的密钥（RFC 8291 的 aes128gcm）：P-256 密钥对加认证密钥，在设备上生成，私钥不出设备
/// （ADR-0003 的“设备的推送密钥”）。存在和通知扩展共享的钥匙串里（经 App Group），扩展用它解开推送；
/// 核心只拿到公钥和认证密钥。
enum PushKeys {
    /// 钥匙串访问组：取 Info.plist 的 `WontonAppGroup`（project.yml 按 bundle ID 注入），换 bundle ID 不会失配。
    private static var accessGroup: String {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "WontonAppGroup") as? String,
              !group.isEmpty else {
            fatalError("WontonAppGroup is missing from Info.plist")
        }
        return group
    }

    private static let service = "app.wonton.ios.push"
    private static let privateKeyAccount = "push.private"
    private static let authAccount = "push.auth"

    /// 登记要用的两样东西：`p256dh` 是未压缩的公钥点（65 字节），`auth` 是认证密钥（16 字节），都是 base64url。
    struct Registration {
        let p256dh: String
        let auth: String
    }

    /// 取已有的，没有就生成。生成过的密钥一直用同一对：换一对等于换设备，核心那边要重新登记。
    static func registration() throws -> Registration {
        let privateKey: P256.KeyAgreement.PrivateKey // gitleaks:allow（类型名被当成密钥值）
        if let existing = read(account: privateKeyAccount), existing.count == 32 {
            privateKey = try P256.KeyAgreement.PrivateKey(rawRepresentation: existing)
        } else {
            privateKey = P256.KeyAgreement.PrivateKey()
            try write(account: privateKeyAccount, data: privateKey.rawRepresentation)
        }
        let auth: Data
        if let existing = read(account: authAccount), existing.count == 16 {
            auth = existing
        } else {
            var bytes = Data(count: 16)
            let status = bytes.withUnsafeMutableBytes {
                SecRandomCopyBytes(kSecRandomDefault, 16, $0.baseAddress.unsafelyUnwrapped)
            }
            guard status == errSecSuccess else { throw CocoaError(.featureUnsupported) }
            auth = bytes
            try write(account: authAccount, data: auth)
        }
        return Registration(
            p256dh: privateKey.publicKey.x963Representation.base64URL, auth: auth.base64URL
        )
    }

    /// 通知扩展解开推送要用的私钥和认证密钥，从共享钥匙串读（主 App 生成时写进去的）。
    static func sharedMaterial() -> (privateKey: P256.KeyAgreement.PrivateKey, auth: Data)? {
        guard
            let privateData = read(account: privateKeyAccount),
            let authData = read(account: authAccount),
            privateData.count == 32, authData.count == 16,
            let privateKey = try? P256.KeyAgreement.PrivateKey(rawRepresentation: privateData)
        else { return nil }
        return (privateKey, authData)
    }

    private static func read(account: String) -> Data? {
        var query = base
        query[kSecAttrAccount as String] = account
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        guard status == errSecSuccess else { return nil }
        return result as? Data
    }

    private static func write(account: String, data: Data) throws {
        var query = base
        query[kSecAttrAccount as String] = account
        let attributes = [kSecValueData as String: data] as CFDictionary
        var status = SecItemUpdate(query as CFDictionary, attributes)
        if status == errSecItemNotFound {
            var add = query
            add[kSecValueData as String] = data
            status = SecItemAdd(add as CFDictionary, nil)
        }
        guard status == errSecSuccess else {
            throw CocoaError(.featureUnsupported, userInfo: [NSLocalizedDescriptionKey: "keychain status \(status)"])
        }
    }

    private static var base: [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccessGroup as String: accessGroup,
            // 锁屏时通知扩展要能读：开机后解锁过一次就一直可读。
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlock
        ]
    }
}

extension Data {
    /// Web Push 用的 base64url：不带填充（RFC 8291）。
    var base64URL: String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
