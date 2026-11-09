import SwiftUI
import WontonClient

/// 插件网页上的“我来操作”（ADR-0024）。只有声明了 MCP 的插件才有。
struct PageTakeover {
    let pluginId: String
    let current: PluginTakeover?
    let send: (Action) -> Void

    var state: TakeoverState? {
        current?.id == pluginId ? current?.state : nil
    }

    func start() {
        send(.startTakeover(id: pluginId))
    }

    func end() {
        send(.endTakeover(id: pluginId))
    }
}

struct TakeoverButton: View {
    let takeover: PageTakeover

    var body: some View {
        switch takeover.state {
        case .operating:
            Button(.takeoverEnd, action: takeover.end)
        case .starting:
            Button(.takeoverStart) {}.disabled(true)
        case nil, .notStarted, .lost:
            Button(.takeoverStart, action: takeover.start)
        }
    }
}

/// 没开始成或者中途断了：Bot 的调用已经不停住了，要让主人知道。
struct TakeoverNotice: View {
    let takeover: PageTakeover

    var body: some View {
        if let problem = takeover.state?.notice {
            WontonNotice(icon: .warning, title: problem.title, lines: problem.nextStep.map { [$0] } ?? []) {
                if let detail = problem.detail { ProblemDetail(text: detail) }
                Button(.takeoverDismiss, action: takeover.end)
                    .buttonStyle(WontonButtonStyle(kind: .secondary))
            }
            .padding(WontonTheme.spacing.s8)
        }
    }
}

extension TakeoverState {
    fileprivate var notice: Problem? {
        switch self {
        case .starting, .operating:
            return nil
        case let .notStarted(problem):
            let plain = problem.problem
            return Problem(
                title: String(localized: .takeoverFailedTitle(plain.title)),
                nextStep: plain.nextStep, detail: plain.detail
            )
        case let .lost(problem) where problem.reason == .transportUnavailable:
            return Problem(title: String(localized: .takeoverLostTransport), detail: problem.message)
        case let .lost(problem):
            let plain = problem.problem
            return Problem(
                title: String(localized: .takeoverLostTitle(plain.title)),
                nextStep: plain.nextStep, detail: plain.detail
            )
        }
    }
}
