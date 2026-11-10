import SnapshotTesting
import SwiftUI
import UIKit
import XCTest
import WontonClient

@testable import Wonton

/// 界面截图回归：真实界面配固定数据，在同一台模拟器、同一个系统版本上逐像素比较。
/// 参考图绑定工作流里的模拟器型号和系统版本，外观、语言、时区由 `SnapshotFixtures` 钉住；
/// 换了模拟器或系统，打 `record-snapshots` 标签整体重录。
@MainActor
final class SnapshotTests: XCTestCase {
    /// 模拟器的尺寸档、外观和字号在录制和比较时都要一样，否则布局和颜色对不上。
    private static let traits = UITraitCollection(traitsFrom: [
        UITraitCollection(displayScale: 3),
        UITraitCollection(userInterfaceStyle: .light),
        UITraitCollection(preferredContentSizeCategory: .large),
        UITraitCollection(horizontalSizeClass: .compact),
        UITraitCollection(verticalSizeClass: .regular),
        UITraitCollection(userInterfaceIdiom: .phone)
    ])

    func testPairingScreen() throws {
        let core = try makeCore()
        assert(SnapshotFixtures.pairing(core: core), testName: "testPairingScreen", line: #line)
    }

    func testChatList() throws {
        let core = try makeCore()
        assert(SnapshotFixtures.chatList(core: core), testName: "testChatList", line: #line)
    }

    func testChatRoom() throws {
        let core = try makeCore()
        assert(SnapshotFixtures.chatRoom(core: core), testName: "testChatRoom", line: #line)
    }

    func testSettings() throws {
        let core = try makeCore()
        assert(SnapshotFixtures.settings(core: core), testName: "testSettings", line: #line)
    }

    /// 逐像素比较；系统字体和矢量图标在同一台模拟器上是确定的，放宽精度要写明原因。
    private func assert(_ view: some View, testName: String, line: UInt) {
        assertSnapshot(
            of: SnapshotFixtures.wrapped(view),
            as: .image(
                drawHierarchyInKeyWindow: true,
                layout: .fixed(width: SnapshotFixtures.size.width, height: SnapshotFixtures.size.height),
                traits: Self.traits
            ),
            testName: testName,
            line: line
        )
    }

    /// 视图要拿核心取不用网络的空列表；数据目录每次删掉，互不影响。
    private func makeCore() throws -> ClientCore {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        addTeardownBlock { try FileManager.default.removeItem(at: directory) }
        return try ClientCore(dataDir: directory.path)
    }
}
