import CryptoKit
import Foundation

/// 推送解开后的两种明文：
/// - 房间消息（ADR-0003 的“接口：推送”，字段同核心的 `DevicePayload`，核心有契约测试锁住字段名）；
/// - 确认卡片直推（ADR-0016）：不是房间消息，没有房间事件，卡片标题在 `title` 里，点开的房间在
///   `origin` 里（核心自己发起的没有，壳落到“等你确认”的入口）。
enum WontonPushPayload {
    struct Message: Decodable {
        let server: String
        let roomId: String
        let eventId: String
        let sender: String?
        let senderDisplayName: String?
        let roomName: String?
    }

    case message(Message)
    case confirmation(Confirmation)
}

struct Confirmation: Decodable {
    let server: String
    let confirmation: ConfirmationCard
}

struct ConfirmationCard: Decodable {
    let id: String
    let title: String
    /// 卡片发起的会话；核心自己发起的卡片没有，点开去“等你确认”的入口。
    let origin: ConfirmationOrigin?
}

struct ConfirmationOrigin: Decodable {
    let roomId: String
}

/// 按明文的形状解析：带 `confirmation` 段的是卡片直推，其余按房间消息。
private struct PayloadShim: Decodable {
    let confirmation: ConfirmationCard?
    let server: String
    let roomId: String?
    let eventId: String?
    let sender: String?
    let senderDisplayName: String?
    let roomName: String?
}

/// RFC 8291 的 aes128gcm 解密：用主 App 生成、存在共享钥匙串里的私钥和认证密钥，解开家里推送代理
/// 加密的通知（中继只见过密文）。只支持单条记录：RFC 8291 第 4 节要求应用服务器单条记录加密。
enum WontonPush {
    enum DecryptError: Error {
        /// 密文不是 aes128gcm 的形状（不是家里的推送代理发的，或者中继的封装变了）。
        case malformed
        /// 解开了但密钥对不上、填充不对或内容读不懂，同样不当成正常通知。
        case undecryptable
    }

    static func decrypt(
        _ encrypted: Data, privateKey: P256.KeyAgreement.PrivateKey, auth: Data
    ) throws -> WontonPushPayload {
        let body = [UInt8](encrypted)
        // 头部：salt（16）| 记录大小（4，大端）| keyid 长度（1）| keyid（应用服务器的未压缩公钥点）。
        guard body.count > 21 else { throw DecryptError.malformed }
        let salt = Data(body[0..<16])
        let recordSize = body[16...19].reduce(0) { $0 << 8 | Int($1) }
        let keyIdLength = Int(body[20])
        guard recordSize >= 18, keyIdLength == 65, body[21] == 0x04,
            body.count >= 21 + keyIdLength + 17
        else { throw DecryptError.malformed }
        let senderKey: P256.KeyAgreement.PublicKey
        do {
            senderKey = try P256.KeyAgreement.PublicKey(x963Representation: Data(body[21..<(21 + keyIdLength)]))
        } catch {
            throw DecryptError.malformed
        }
        // RFC 8291 第 4 节：IKM 由 ECDH 秘密（盐是认证密钥）和 "WebPush: info" 加两把公钥展开而来；
        // CEK 和 nonce 再由它以 salt 为盐展开。
        let shared = try privateKey.sharedSecretFromKeyAgreement(with: senderKey)
        let ecdh = shared.withUnsafeBytes { Data($0) }
        let keyInfo = Data("WebPush: info".utf8) + [0]
            + privateKey.publicKey.x963Representation + senderKey.x963Representation
        let ikm = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: SymmetricKey(data: ecdh), salt: auth, info: keyInfo, outputByteCount: 32
        )
        let cek = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: ikm, salt: salt,
            info: Data("Content-Encoding: aes128gcm\u{0}".utf8), outputByteCount: 16
        )
        let nonce = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: ikm, salt: salt,
            info: Data("Content-Encoding: nonce\u{0}".utf8), outputByteCount: 12
        )
        .withUnsafeBytes { Data($0) }
        // 单条记录，记录序号为 0，nonce 不用异或；nonce 从 HKDF 来、进 SealedBox 的头部。
        let sealed: AES.GCM.SealedBox
        do {
            sealed = try AES.GCM.SealedBox(
                nonce: AES.GCM.Nonce(data: nonce), ciphertext: Data(body[(21 + keyIdLength)...]).dropLast(16),
                tag: Data(body[(21 + keyIdLength)...]).suffix(16)
            )
        } catch {
            throw DecryptError.malformed
        }
        var plaintext: Data
        do {
            plaintext = try AES.GCM.open(sealed, using: cek)
        } catch {
            throw DecryptError.undecryptable
        }
        // 填充分隔符是记录的最后一个字节，单条记录必须是 0x02（RFC 8291 第 5 节）；它前面的 0x00 是填充。
        guard plaintext.popLast() == 0x02 else { throw DecryptError.undecryptable }
        while plaintext.last == 0x00 { plaintext.removeLast() }
        // 明文的字段名是 snake_case 的契约（核心锁住），Swift 侧用驼峰。
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        guard let shim = try? decoder.decode(PayloadShim.self, from: plaintext) else {
            throw DecryptError.undecryptable
        }
        if let card = shim.confirmation {
            return .confirmation(.init(server: shim.server, confirmation: card))
        }
        guard let roomId = shim.roomId, let eventId = shim.eventId else {
            throw DecryptError.undecryptable
        }
        return .message(.init(
            server: shim.server, roomId: roomId, eventId: eventId,
            sender: shim.sender, senderDisplayName: shim.senderDisplayName, roomName: shim.roomName
        ))
    }
}
