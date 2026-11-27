import CryptoKit
import XCTest

@testable import Wonton

/// 通知扩展的解密：RFC 8291 附录 A 的测试向量，和核心（wonton-core 的 push 测试）用的是同一组。
@MainActor
final class PushDecryptTests: XCTestCase {
    /// RFC 8291 第 5 节例子的明文。
    private static let plaintext = "When I grow up, I want to be a watermelon"
    private static let authSecret = "BTBZMqHH6r4Tts7J_aSIgg" // gitleaks:allow（RFC 8291 的公开测试向量）
    private static let uaPrivate = "q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94"
    private static let salt = "DGv6ra1nlYgDCS1FRnbzlw"
    private static let senderPrivate = "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw"

    /// RFC 8291 第 5 节里那份完整的密文（keyid 是应用服务器的公钥）。
    private static let ciphertext = """
        DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAm\
        S6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_\
        Qulcy4a-fN
        """

    func testRfcVectorDecryptsToThePlaintext() throws {
        // RFC 8291 附录 A 的中间值全在这份密文里：ECDH、HKDF 链、AES-GCM 和填充分隔符都对上才算过。
        XCTAssertEqual(try decryptRaw(Self.ciphertext), Data(Self.plaintext.utf8))
        // 向量的明文不是 JSON：扩展这一层要把它当解不开（显示通用的“有新消息”），不能崩。
        XCTAssertThrowsError(try WontonPush.decrypt(
            base64URL(Self.ciphertext),
            privateKey: P256.KeyAgreement.PrivateKey(rawRepresentation: base64URL(Self.uaPrivate)),
            auth: base64URL(Self.authSecret)
        ))
    }

    /// 核心发来的明文（`DevicePayload`）能解出来，字段名是两边的契约（ADR-0003 的“接口：推送”）。
    func testDevicePayloadRoundTrips() throws {
        let json = """
            {"server":"server-endpoint-id","room_id":"!bot:wonton.localhost","event_id":"$reply-1",
             "sender":"@hermes:wonton.localhost","sender_display_name":"Hermes","room_name":"Hermes 主房间"}
            """
        let (encrypted, keys) = try encrypt(json)
        guard case .message(let payload) = try WontonPush.decrypt(encrypted, privateKey: keys.ua, auth: keys.auth) else {
            return XCTFail("房间消息的明文要解成 .message")
        }
        XCTAssertEqual(payload.server, "server-endpoint-id")
        XCTAssertEqual(payload.roomId, "!bot:wonton.localhost")
        XCTAssertEqual(payload.eventId, "$reply-1")
        XCTAssertEqual(payload.sender, "@hermes:wonton.localhost")
        XCTAssertEqual(payload.senderDisplayName, "Hermes")
        XCTAssertEqual(payload.roomName, "Hermes 主房间")
    }

    /// 核心直推的确认卡片（ADR-0016）：带 `confirmation` 段的明文解成 .confirmation，没有来源会话的卡片
    /// `origin` 为 nil（点开后壳落到收件箱）。
    func testConfirmationPushParses() throws {
        let withOrigin = """
            {"server":"srv","confirmation":{"id":"c1","title":"claw 想安装 Grafana",\
            "expires_at_ms":42,"origin":{"room_id":"!room:test"}}}
            """
        let (encrypted, keys) = try encrypt(withOrigin)
        guard case .confirmation(let direct) = try WontonPush.decrypt(encrypted, privateKey: keys.ua, auth: keys.auth)
        else { return XCTFail("确认卡片的明文要解成 .confirmation") }
        XCTAssertEqual(direct.server, "srv")
        XCTAssertEqual(direct.confirmation.id, "c1")
        XCTAssertEqual(direct.confirmation.title, "claw 想安装 Grafana")
        XCTAssertEqual(direct.confirmation.origin?.roomId, "!room:test")

        let withoutOrigin = """
            {"server":"srv","confirmation":{"id":"c2","title":"服务器配对","expires_at_ms":7}}
            """
        let (encrypted2, keys2) = try encrypt(withoutOrigin)
        guard case .confirmation(let direct2) = try WontonPush.decrypt(encrypted2, privateKey: keys2.ua, auth: keys2.auth)
        else { return XCTFail("确认卡片的明文要解成 .confirmation") }
        XCTAssertNil(direct2.confirmation.origin)
    }

    func testCorruptedCiphertextIsRejected() throws {
        let (encrypted, keys) = try encrypt("x")
        var corrupted = encrypted
        corrupted[corrupted.count - 1] ^= 0xFF
        XCTAssertThrowsError(try WontonPush.decrypt(corrupted, privateKey: keys.ua, auth: keys.auth))
        // 不是家里发的（胡乱的字节、keyid 长度不对）也不解。
        XCTAssertThrowsError(try WontonPush.decrypt(Data([0, 1, 2]), privateKey: keys.ua, auth: keys.auth))
    }

    /// 只解到原始字节：RFC 向量的明文不是 JSON。
    private func decryptRaw(_ base64: String) throws -> Data {
        let keys = (
            ua: try P256.KeyAgreement.PrivateKey(rawRepresentation: base64URL(Self.uaPrivate)),
            auth: base64URL(Self.authSecret)
        )
        let body = [UInt8](base64URL(base64))
        let keyIdLength = Int(body[20])
        let senderKey = try P256.KeyAgreement.PublicKey(x963Representation: Data(body[21..<(21 + keyIdLength)]))
        let shared = try keys.ua.sharedSecretFromKeyAgreement(with: senderKey)
        let ecdh = shared.withUnsafeBytes { Data($0) }
        let keyInfo = Data("WebPush: info".utf8) + [0] + keys.ua.publicKey.x963Representation
            + senderKey.x963Representation
        let ikm = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: SymmetricKey(data: ecdh), salt: keys.auth, info: keyInfo, outputByteCount: 32
        )
        let cek = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: ikm, salt: Data(body[0..<16]),
            info: Data("Content-Encoding: aes128gcm\u{0}".utf8), outputByteCount: 16
        )
        let nonce = HKDF<SHA256>.deriveKey(
            inputKeyMaterial: ikm, salt: Data(body[0..<16]),
            info: Data("Content-Encoding: nonce\u{0}".utf8), outputByteCount: 12
        )
        .withUnsafeBytes { Data($0) }
        let record = Data(body[(21 + keyIdLength)...])
        let sealed = try AES.GCM.SealedBox(
            nonce: AES.GCM.Nonce(data: nonce), ciphertext: record.dropLast(16), tag: record.suffix(16)
        )
        var plaintext = try AES.GCM.open(sealed, using: cek)
        XCTAssertEqual(plaintext.popLast(), 0x02, "RFC 的填充分隔符是 0x02")
        while plaintext.last == 0x00 { plaintext.removeLast() }
        return plaintext
    }

    /// 测试用的加密（RFC 8291 第 5 节，单条记录，salt 固定成 RFC 的那个）。
    private func encrypt(_ json: String) throws -> (Data, (ua: P256.KeyAgreement.PrivateKey, auth: Data)) {
        let ua = P256.KeyAgreement.PrivateKey()
        let sender = try P256.KeyAgreement.PrivateKey(rawRepresentation: base64URL(Self.senderPrivate))
        let auth = base64URL(Self.authSecret)
        let salt = base64URL(Self.salt)
        let shared = try sender.sharedSecretFromKeyAgreement(with: ua.publicKey)
        let ecdh = shared.withUnsafeBytes { Data($0) }
        let keyInfo = Data("WebPush: info".utf8) + [0] + ua.publicKey.x963Representation
            + sender.publicKey.x963Representation
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
        var padded = Data(json.utf8)
        padded.append(0x02)
        let sealed = try AES.GCM.seal(padded, using: cek, nonce: AES.GCM.Nonce(data: nonce))
        let rs = padded.count + 16 + 1
        var body = salt
        body.append(contentsOf: [0, 0, UInt8((rs >> 8) & 0xFF), UInt8(rs & 0xFF)])
        body.append(65)
        body.append(contentsOf: sender.publicKey.x963Representation)
        body.append(contentsOf: sealed.ciphertext + sealed.tag)
        return (body, (ua, auth))
    }

    private func base64URL(_ text: String) -> Data {
        var base64 = text.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        if base64.count % 4 != 0 { base64.append(String(repeating: "=", count: 4 - base64.count % 4)) }
        return Data(base64Encoded: base64)!
    }
}
