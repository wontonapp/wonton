#if DEBUG
import SwiftUI
import WontonClient

/// 界面测试的出错页夹具：启动参数带 `wonton-problem-preview` 时生效（同 `AgentRunPreview` 的做法）。
/// 摆的是几种失败类别（`notReady`、`invalidInput`、`serverNotPaired`、`peerPublicAccessOff`）经
/// `problem(detail:)` 换算出来的样子：正文只说一句壳自己的话，核心的原文收在默认收起的“详情”里（AGENTS.md“界面设计”）。
enum ProblemPreview {
    static var active: Bool {
        ProcessInfo.processInfo.arguments.contains("wonton-problem-preview")
    }

    static var root: some View {
        problemScreen
    }

    /// 核心原文：带错误码和 SDK 味的一句，正文里不该直接铺开。
    static let notReadyDetail = "The sync session is not ready yet (M_UNKNOWN, since 14:02)"
    static let invalidInputDetail = "Can't build a reply to this message: M_NOT_FOUND"

    static let serverNotPairedDetail = "451: no federation route to @sam:sam5e8f.wonton"
    static let peerPublicAccessOffDetail = "503 MatrixUnavailable (peer public access off)"
    private static var problemScreen: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: WontonTheme.spacing.s32) {
                problem(ChatFailure.notReady, notReadyDetail, id: "problemFixture-notReady")
                Divider()
                problem(ChatFailure.serverNotPaired, Self.serverNotPairedDetail, id: "problemFixture-serverNotPaired")
                Divider()
                problem(
                    ChatFailure.peerPublicAccessOff, Self.peerPublicAccessOffDetail,
                    id: "problemFixture-peerPublicAccessOff"
                )
                Divider()
                problem(ChatFailure.invalidInput, invalidInputDetail, id: "problemFixture-invalidInput")
            }
            .padding(WontonTheme.spacing.s24)
        }
    }

    private static func problem(_ failure: ChatFailure, _ detail: String, id: String) -> some View {
        ProblemView(problem: failure.problem(detail: detail), retry: {})
            .accessibilityElement(children: .contain)
            .accessibilityIdentifier(id)
    }
}
#endif
