import XCTest
import WontonClient
@testable import Wonton

@MainActor
final class PairingLinkTests: XCTestCase {
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
        XCTAssertEqual(core.home().pairing, .inProgress)
    }

    func testLinkCannotReplaceAnActivePairingAttempt() async throws {
        let core = try makeCore()
        let home = HomeModel(core: core)
        let original = pairingURL()
        XCTAssertTrue(home.prefillPairing(from: original))
        XCTAssertNil(home.pair(code: home.pairingCode, deviceName: "Test phone"))
        XCTAssertEqual(core.home().pairing, .inProgress)

        XCTAssertFalse(home.prefillPairing(from: pairingURL(domain: "other.example")))
        XCTAssertEqual(home.pairingCode, original.absoluteString)
        XCTAssertEqual(core.home().pairing, .inProgress)
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
