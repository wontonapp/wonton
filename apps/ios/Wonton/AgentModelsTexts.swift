import Foundation
import WontonClient

extension AgentModelsProblem {
    /// 类别写成一句人话和能做什么；核心的原文收进默认收起的“详情”。
    var problem: Problem {
        let (title, nextStep) = reason.texts
        let lines = [message, detail ?? ""].filter { !$0.isEmpty }
        return Problem(title: title, nextStep: nextStep, detail: lines.isEmpty ? nil : lines.joined(separator: "\n"))
    }
}

private extension AgentModelsReason {
    var texts: (title: String, nextStep: String) {
        switch self {
        case .transport:
            (
                String(localized: .Problem2.problemTransportUnavailableTitle),
                String(localized: .Problem3.problemTransportUnavailableNextStep)
            )
        case .forbidden:
            (
                String(localized: .AgentModels0.agentModelsProblemForbidden),
                String(localized: .AgentModels2.agentModelsProblemForbiddenNext)
            )
        case .notInstalled:
            (
                String(localized: .AgentModels1.agentModelsProblemNotInstalled),
                String(localized: .AgentModels2.agentModelsProblemNotInstalledNext)
            )
        case .notReady:
            (
                String(localized: .AgentModels2.agentModelsProblemNotReady),
                String(localized: .AgentModels2.agentModelsProblemNotReadyNext)
            )
        case .unsupported:
            (
                String(localized: .AgentModels1.agentModelsProblemUnsupported),
                String(localized: .AgentModels2.agentModelsProblemUnsupportedNext)
            )
        case .notFound:
            (
                String(localized: .AgentModels2.agentModelsProblemNotFound),
                String(localized: .AgentModels2.agentModelsProblemNotFoundNext)
            )
        case .invalidInput:
            (
                String(localized: .AgentModels1.agentModelsProblemInvalidInput),
                String(localized: .AgentModels0.agentModelsProblemInvalidInputNext)
            )
        case .upstream:
            (
                String(localized: .AgentModels0.agentModelsProblemUpstream),
                String(localized: .Problem1.problemUpstreamNext)
            )
        case .protocol:
            (
                String(localized: .Problem0.problemPluginProtocolTitle),
                String(localized: .Problem0.problemPluginProtocolNextStep)
            )
        case .other:
            (
                String(localized: .AgentModels1.agentModelsProblemOther),
                String(localized: .Problem1.problemUpstreamNext)
            )
        }
    }
}

extension ModelTestResult {
    /// 测试没通的一句话；通了的由调用方写（带不带耗时）。
    var failureText: String {
        switch self {
        case .ok: String(localized: .AgentModels0.agentModelsTestOk)
        case .rejected: String(localized: .AgentModels1.agentModelsTestRejected)
        case .unreachable: String(localized: .AgentModels2.agentModelsTestUnreachable)
        case .upstream: String(localized: .AgentModels1.agentModelsTestUpstream)
        case .noModel: String(localized: .AgentModels1.agentModelsTestNoModel)
        case .other: String(localized: .AgentModels2.agentModelsTestOther)
        }
    }
}

extension ModelTest {
    var text: String {
        guard result == .ok else { return result.failureText }
        guard let latencyMs else { return String(localized: .AgentModels0.agentModelsTestOk) }
        let seconds = (Double(latencyMs) / 1_000).formatted(.number.precision(.fractionLength(1)))
        return String(localized: .AgentModels2.agentModelsTestOkIn(seconds))
    }
}

extension LoginState {
    /// 登录没成的一句话；还在等和登好了的不显示。
    var outcomeText: String? {
        switch self {
        case .pending, .approved: nil
        case .denied: String(localized: .AgentModels0.agentModelsLoginDenied)
        case .expired: String(localized: .AgentModels1.agentModelsLoginExpired)
        case .failed, .other: String(localized: .AgentModels0.agentModelsLoginFailed)
        }
    }
}

extension AgentModelsViewModel {
    /// 服务商的名字：在用的那家可能被搜索词滤掉了，找不到时退回插件给的 ID。
    func providerName(_ id: String) -> String {
        let rows = recommended + more + (provider.map { [$0.row] } ?? [])
        return rows.first { $0.id == id }?.name ?? id
    }
}
