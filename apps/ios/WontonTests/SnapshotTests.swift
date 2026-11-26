#if DEBUG
import SnapshotTesting
import SwiftUI
import UIKit
import XCTest
import WontonClient

@testable import Wonton

/// 界面截图回归：真实界面配固定数据，在同一型号的模拟器、同一个系统版本上逐像素比较。
/// 参考图绑定模拟器型号标识和系统版本，外观、语言、时区由 `SnapshotFixtures` 钉住；别的型号或系统上
/// 字体、圆角、系统栏的画法都不一样，不拿像素硬比：本机跳过、CI 上失败（见 `setUp`）。
/// 换了型号或系统，打 `record-snapshots` 标签整体重录。
@MainActor
final class SnapshotTests: XCTestCase {

    /// 参考图是在哪个型号的模拟器、哪个系统版本上录的。CI 用 `TEST_RUNNER_SNAPSHOT_MODEL`、
    /// `TEST_RUNNER_SNAPSHOT_OS` 把工作流挑的型号、运行时交给测试进程（xcodebuild 只把 `TEST_RUNNER_`
    /// 开头的变量交给模拟器里的 App），工作流是唯一来源。型号是 `SIMULATOR_MODEL_IDENTIFIER` 的取值，
    /// `iPhone18,3` 就是 iPhone 17；运行时按机器挑（有 27.1 用 27.1，没有用机器上最新的 iOS 版本）。
    /// 本机没有这两个变量时用当前这台机器的组合，参考图那个运行时上照样能比；换机器或改参考图要打 record-snapshots 重录。
    private enum ReferenceSimulator {
        private static let environment = ProcessInfo.processInfo.environment
        private static let runningOS = ProcessInfo.processInfo.operatingSystemVersion
        static let model = environment["SNAPSHOT_MODEL"] ?? environment["SIMULATOR_MODEL_IDENTIFIER"] ?? "iPhone18,3"
        static let osVersion =
            environment["SNAPSHOT_OS"] ?? "\(runningOS.majorVersion).\(runningOS.minorVersion)"
        /// CI 上设 `TEST_RUNNER_SNAPSHOT_ENFORCE=1`：不符就失败而不是跳过，免得工作流和参考图漂开之后 CI 一路绿。
        static let enforced = environment["SNAPSHOT_ENFORCE"] == "1"
    }

    /// 运行时不符：本机整类跳过、CI 上整类失败，原因里写清参考版本和当前版本。
    /// 界面按 `SnapshotFixtures.size`（390×844）固定尺寸渲染，机型只影响系统栏以外的少量画法，型号不上锁。
    override func setUp() async throws {
        let environment = ProcessInfo.processInfo.environment
        if ReferenceSimulator.enforced, environment["SNAPSHOT_OS"] == nil {
            throw ReferenceRuntimeMismatch("CI 上没收到 TEST_RUNNER_SNAPSHOT_OS，参考运行时没交给测试进程。")
        }
        let model = environment["SIMULATOR_MODEL_IDENTIFIER"] ?? "不在模拟器里"
        let version = UIDevice.current.systemVersion
        let simulator = environment["SIMULATOR_DEVICE_NAME"].map { "（模拟器 \($0)）" } ?? ""
        guard Self.majorMinor(version) == ReferenceSimulator.osVersion else {
            throw Self.skipOrFail(
                "参考图是在 iOS \(ReferenceSimulator.osVersion) 上录的（\(ReferenceSimulator.model)），"
                    + "当前是 \(model)/iOS \(version)\(simulator)，像素比对没有意义；"
                    + "在参考运行时上跑，或用 record-snapshots 标签重录。"
            )
        }
    }

    /// 本机没有 CI 的信号就跳过，CI 上（`TEST_RUNNER_SNAPSHOT_ENFORCE=1`）失败。
    private static func skipOrFail(_ message: String) -> Error {
        if ReferenceSimulator.enforced {
            return ReferenceRuntimeMismatch(message)
        }
        return XCTSkip(message)
    }

    /// 从上到下写的错因，XCTest 原样打进日志。
    private struct ReferenceRuntimeMismatch: Error, CustomStringConvertible {
        let description: String

        init(_ description: String) {
            self.description = description
        }
    }

    /// 只比主次版本：补丁版本不动渲染，参考图不用跟着 x.y.z 的三个数字走。
    private static func majorMinor(_ version: String) -> String {
        version.split(separator: ".").prefix(2).joined(separator: ".")
    }

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

    func testWideChat() throws {
        let core = try makeCore()
        let traits = UITraitCollection(traitsFrom: [
            UITraitCollection(displayScale: 2),
            UITraitCollection(userInterfaceStyle: .light),
            UITraitCollection(preferredContentSizeCategory: .large),
            UITraitCollection(horizontalSizeClass: .regular),
            UITraitCollection(verticalSizeClass: .regular),
            UITraitCollection(userInterfaceIdiom: .pad)
        ])
        assertSnapshot(
            of: SnapshotFixtures.wrapped(SnapshotFixtures.wideChat(core: core)),
            as: .image(drawHierarchyInKeyWindow: true, layout: .fixed(width: 1194, height: 834), traits: traits)
        )
    }

    func testChatRoom() throws {
        let core = try makeCore()
        assert(SnapshotFixtures.chatRoom(core: core), testName: "testChatRoom", line: #line)
    }

    func testSettings() throws {
        let core = try makeCore()
        assert(SnapshotFixtures.settings(core: core), testName: "testSettings", line: #line)
    }

    func testPluginProblem() throws {
        assert(SnapshotFixtures.pluginProblem(), testName: "testPluginProblem", line: #line)
    }

    func testShareSheet() throws {
        assert(SnapshotFixtures.shareSheet(), testName: "testShareSheet", line: #line)
    }

    // 小组件（ADR-0100）：假快照见 SnapshotFixtures.entrySnapshot；尺寸取 iPhone 上常见的档，
    // 参考图绑定本工作流的模拟器，重录打 record-snapshots 标签。
    func testWidgetRecentMedium() throws {
        assert(
            SnapshotFixtures.widget(kind: .recent, family: .systemMedium),
            size: CGSize(width: 329, height: 155), testName: "testWidgetRecentMedium", line: #line
        )
    }

    func testWidgetRecentInline() throws {
        assert(
            SnapshotFixtures.widget(kind: .recent, family: .accessoryInline),
            size: CGSize(width: 329, height: 48), testName: "testWidgetRecentInline", line: #line
        )
    }

    func testWidgetStatusSmall() throws {
        assert(
            SnapshotFixtures.widget(kind: .status, family: .systemSmall),
            size: CGSize(width: 158, height: 158), testName: "testWidgetStatusSmall", line: #line
        )
    }

    func testWidgetStatusRectangular() throws {
        assert(
            SnapshotFixtures.widget(kind: .status, family: .accessoryRectangular),
            size: CGSize(width: 155, height: 110), testName: "testWidgetStatusRectangular", line: #line
        )
    }

    func testWidgetAskSmall() throws {
        assert(
            SnapshotFixtures.widget(kind: .ask, family: .systemSmall),
            size: CGSize(width: 158, height: 158), testName: "testWidgetAskSmall", line: #line
        )
    }

    func testWidgetAskCircular() throws {
        assert(
            SnapshotFixtures.widget(kind: .ask, family: .accessoryCircular),
            size: CGSize(width: 155, height: 155), testName: "testWidgetAskCircular", line: #line
        )
    }

    /// 未配对时只写一句“打开云吞完成配对”，点了进 App（ADR-0100 决策 1）。
    func testWidgetNotPaired() throws {
        assert(
            SnapshotFixtures.widget(kind: .recent, family: .systemMedium, account: .notPaired),
            size: CGSize(width: 329, height: 155), testName: "testWidgetNotPaired", line: #line
        )
    }

    /// 侧边栏容器要把页面的底铺进系统栏那两条（状态栏、Home 条）：收起时整屏是页面的底色，
    /// 打开时露出来的那一条连同圆角也铺到屏幕上下边。主内容故意不画底，底只能来自容器。
    func testSidebarClosedFillsSafeArea() throws {
        let view = SnapshotFixtures.sidebar(isOpen: false)
        assertWithSafeArea(view, testName: "testSidebarClosedFillsSafeArea", line: #line)
    }

    func testSidebarOpenFillsSafeArea() throws {
        let view = SnapshotFixtures.sidebar(isOpen: true)
        assertWithSafeArea(view, testName: "testSidebarOpenFillsSafeArea", line: #line)
    }

    /// 截图用的窗口没有系统栏，安全区是零；这里按带 Home 条的 iPhone 补上，安全区的画法才进得了参考图。
    private func assertWithSafeArea(_ view: some View, testName: String, line: UInt) {
        let controller = UIHostingController(rootView: SnapshotFixtures.wrapped(view))
        controller.additionalSafeAreaInsets = SnapshotFixtures.safeArea
        assertSnapshot(
            of: controller,
            as: .image(drawHierarchyInKeyWindow: true, size: SnapshotFixtures.size, traits: Self.traits),
            testName: testName,
            line: line
        )
    }

    /// 逐像素比较；系统字体和矢量图标在同一台模拟器上是确定的，放宽精度要写明原因。
    private func assert(_ view: some View, size: CGSize = SnapshotFixtures.size, testName: String, line: UInt) {
        assertSnapshot(
            of: SnapshotFixtures.wrapped(view),
            as: .image(
                drawHierarchyInKeyWindow: true,
                layout: .fixed(width: size.width, height: size.height),
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
#endif
