import WontonClient

extension ModelProviderProblem {
    var onboardingProblem: Problem {
        let title: String
        let nextStep: String
        switch reason {
        case .keyRejected:
            title = String(localized: .onboardingProblemModelKeyRejected)
            nextStep = String(localized: .onboardingProblemModelKeyRejectedNext)
        case .providerUnreachable:
            title = String(localized: .onboardingProblemModelProviderUnreachable)
            nextStep = String(localized: .onboardingProblemModelProviderUnreachableNext)
        case .gatewayNotReady:
            title = String(localized: .onboardingProblemModelGatewayNotReady)
            nextStep = String(localized: .onboardingProblemModelGatewayNotReadyNext)
        case .transportUnavailable:
            title = String(localized: .onboardingProblemModelTransportUnavailable)
            nextStep = String(localized: .onboardingProblemModelTransportUnavailableNext)
        case .unauthorized:
            title = String(localized: .onboardingProblemModelUnauthorized)
            nextStep = String(localized: .onboardingProblemModelUnauthorizedNext)
        case .upstream:
            title = String(localized: .onboardingProblemModelUpstream)
            nextStep = String(localized: .onboardingProblemModelUpstreamNext)
        case .protocol:
            title = String(localized: .onboardingProblemModelProtocol)
            nextStep = String(localized: .onboardingProblemModelProtocolNext)
        }
        return Problem(title: title, nextStep: nextStep, detail: message)
    }
}

extension StoreProblem {
    var onboardingProblem: Problem {
        let title: String
        switch reason {
        case .unreachable:
            title = String(localized: .onboardingProblemStoreUnreachable)
        case .timeout:
            title = String(localized: .onboardingProblemStoreTimeout)
        case .rateLimited:
            title = String(localized: .onboardingProblemStoreRateLimited)
        case .invalidIndex:
            title = String(localized: .onboardingProblemStoreInvalidIndex)
        case .indexTooNew:
            title = String(localized: .onboardingProblemStoreIndexTooNew)
        case .upstream:
            title = String(localized: .onboardingProblemStoreUpstream)
        case .incomplete:
            title = String(localized: .onboardingProblemStoreIncomplete)
        case .disabled:
            title = String(localized: .onboardingProblemStoreDisabled)
        case .other:
            title = String(localized: .onboardingProblemStoreOther)
        }
        return Problem(title: title, nextStep: String(localized: .onboardingProblemStoreNext), detail: details)
    }
}
