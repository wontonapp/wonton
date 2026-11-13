import XCTest
import WontonClient
@testable import Wonton

@MainActor
final class PairingLinkTests: XCTestCase {
    func testRefusedScanKeepsRawReasonInDetailsAndCanBeRetried() throws {
        let core = try makeCore()
        let home = HomeModel(core: core)
        let code = "wonton://pair?v=0"

        let refusal = try XCTUnwrap(home.pair(code: code, deviceName: "Test phone"))
        XCTAssertNil(refusal.nextStep)
        XCTAssertFalse(try XCTUnwrap(refusal.detail).isEmpty)
        XCTAssertNil(home.viewModel.pairing)
        home.dismissPairingFailure()
        XCTAssertNil(home.viewModel.pairing)
        XCTAssertNotNil(home.pair(code: code, deviceName: "Test phone"))
    }

    func testPairLinkOnlyPrefillsUntilConfirmation() async throws {
        let core = try makeCore()
        let home = HomeModel(core: core)
        let code = pairingURL()
        let before = core.home()
        let servers = core.servers()

        XCTAssertTrue(home.prefillPairing(from: code))

        XCTAssertEqual(home.pairingCode, code.absoluteString)
        XCTAssertEqual(core.home(), before)
        XCTAssertEqual(core.servers(), servers)
        XCTAssertNil(home.pair(code: home.pairingCode, deviceName: "Test phone"))
        XCTAssertTrue(core.home().pairing.isPairing, "点确认后核心应报正在配对：\(String(describing: core.home().pairing))")
        XCTAssertTrue(home.viewModel.pairing.isPairing)
        home.cancelPairing()
        XCTAssertNil(home.viewModel.pairing)
    }

    func testLinkCannotReplaceAnActivePairingAttempt() async throws {
        let core = try makeCore()
        let home = HomeModel(core: core)
        let original = pairingURL()
        XCTAssertTrue(home.prefillPairing(from: original))
        XCTAssertNil(home.pair(code: home.pairingCode, deviceName: "Test phone"))
        XCTAssertTrue(core.home().pairing.isPairing)

        XCTAssertFalse(home.prefillPairing(from: pairingURL(domain: "other.example")))
        XCTAssertEqual(home.pairingCode, original.absoluteString)
        XCTAssertTrue(core.home().pairing.isPairing)
    }

    private func makeCore() throws -> ClientCore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        addTeardownBlock { try FileManager.default.removeItem(at: directory) }
        return try ClientCore(dataDir: directory.path)
    }

    private func pairingURL(domain: String = "home.example") -> URL {
        var url = URLComponents(string: "wonton://pair")!
        url.queryItems = [
            URLQueryItem(name: "v", value: "0"),
            URLQueryItem(name: "id", value: "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"),
            URLQueryItem(name: "addr", value: "127.0.0.1:9"),
            URLQueryItem(name: "domain", value: domain),
            URLQueryItem(name: "core", value: "http://\(domain)/"),
            URLQueryItem(name: "secret", value: String(repeating: "ab", count: 16))
        ]
        return url.url!
    }
}

extension PairingStatus? {
    /// 正在配对：进展带上了时间和步骤，判断用模式匹配。
    var isPairing: Bool {
        if case .inProgress = self { return true }
        return false
    }
}
