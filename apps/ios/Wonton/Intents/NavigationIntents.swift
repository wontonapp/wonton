import AppIntents
import Foundation
import WontonClient

struct AskAgentIntent: AppIntent {
    static let title: LocalizedStringResource = "问 Bot"
    static let description = IntentDescription("把一句话发给默认 Agent，并打开私聊。")
    static let openAppWhenRun = true

    @Parameter(title: "想问的话", requestValueDialog: "你想问 Bot 什么？")
    var text: String

    static var parameterSummary: some ParameterSummary {
        Summary("问 Bot：\(\.$text)")
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        let text = text
        try shared().request { try await $0.askAgent(text) }
        return .result()
    }
}

struct OpenPageIntent: AppIntent {
    static let title: LocalizedStringResource = "打开页面"
    static let description = IntentDescription("打开云吞里的页面。")
    static let openAppWhenRun = true

    @Parameter(title: "页面")
    var page: WontonPageEntity

    static var parameterSummary: some ParameterSummary {
        Summary("打开 \(\.$page)")
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        guard let link = parseAppLink(url: page.id), case .tab = link else {
            throw IntentProblem(CoreError.InvalidInput(reason: "这个页面链接无效，请重新选择页面。"))
        }
        try shared().request { try await $0.open(link) }
        return .result()
    }
}

struct NewChatIntent: AppIntent {
    static let title: LocalizedStringResource = "新建会话"
    static let description = IntentDescription("打开云吞的发起私聊页面。")
    static let openAppWhenRun = true

    @MainActor
    func perform() async throws -> some IntentResult {
        try shared().request { try await $0.open(.newChat) }
        return .result()
    }
}

@MainActor
private func shared() throws -> AppNavigation {
    do {
        return try AppNavigation.requireShared()
    } catch {
        throw IntentProblem(error)
    }
}

/// 使用 App 现有的错误类别文案，不把核心的技术描述直接交给 Siri。
struct IntentProblem: LocalizedError, CustomLocalizedStringResourceConvertible {
    let message: String

    init(_ error: any Error) {
        let problem = error.problem
        message = (error as? IntentProblem)?.message
            ?? [problem.title, problem.nextStep].compactMap { $0 }.joined(separator: "。")
    }

    init(problem: Problem) {
        message = [problem.title, problem.nextStep].compactMap { $0 }.joined(separator: "。")
    }

    var errorDescription: String? { message }
    var localizedStringResource: LocalizedStringResource { "\(message)" }
}
