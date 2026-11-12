import AppIntents
import Foundation
import WontonClient

struct AskAgentIntent: AppIntent {
    static let title: LocalizedStringResource = "Ask a Bot"
    static let description = IntentDescription("Sends a sentence to your default agent and opens the direct chat.")
    static let openAppWhenRun = true

    @Parameter(
        title: LocalizedStringResource("Message"),
        requestValueDialog: IntentDialog("What do you want to ask the bot?")
    )
    var text: String

    static var parameterSummary: some ParameterSummary {
        Summary("Ask a bot: \(\.$text)")
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        let text = text
        try shared().request { try await $0.askAgent(text) }
        return .result()
    }
}

struct OpenPageIntent: AppIntent {
    static let title: LocalizedStringResource = "Open Page"
    static let description = IntentDescription("Opens a page in Wonton.")
    static let openAppWhenRun = true

    @Parameter(title: LocalizedStringResource("Page"))
    var page: WontonPageEntity

    static var parameterSummary: some ParameterSummary {
        Summary("Open \(\.$page)")
    }

    @MainActor
    func perform() async throws -> some IntentResult {
        guard let link = parseAppLink(url: page.id), case .tab = link else {
            throw IntentProblem(CoreError.InvalidInput(reason: String(localized: .Shortcuts.shortcutsInvalidPageLink)))
        }
        try shared().request { try await $0.open(link) }
        return .result()
    }
}

struct NewChatIntent: AppIntent {
    static let title: LocalizedStringResource = "New Chat"
    static let description = IntentDescription("Opens Wonton's start-a-chat page.")
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
            ?? [problem.title, problem.nextStep].compactMap { $0 }
            .joined(separator: String(localized: .Common.commonSentenceSeparator))
    }

    init(problem: Problem) {
        message = [problem.title, problem.nextStep].compactMap { $0 }
            .joined(separator: String(localized: .Common.commonSentenceSeparator))
    }

    var errorDescription: String? { message }
    var localizedStringResource: LocalizedStringResource { "\(message)" }
}
