import WontonClient

extension ModelProviderProblem {
    var onboardingProblem: Problem {
        let title: String
        let nextStep: String
        switch reason {
        case .keyRejected:
            title = String(localized: .Onboarding4.onboardingProblemModelKeyRejected)
            nextStep = String(localized: .Onboarding5.onboardingProblemModelKeyRejectedNext)
        case .providerUnreachable:
            title = String(localized: .Onboarding0.onboardingProblemModelProviderUnreachable)
            nextStep = String(localized: .Onboarding3.onboardingProblemModelProviderUnreachableNext)
        case .gatewayNotReady:
            title = String(localized: .Onboarding0.onboardingProblemModelGatewayNotReady)
            nextStep = String(localized: .Onboarding0.onboardingProblemModelGatewayNotReadyNext)
        case .transportUnavailable:
            title = String(localized: .Onboarding5.onboardingProblemModelTransportUnavailable)
            nextStep = String(localized: .Onboarding2.onboardingProblemModelTransportUnavailableNext)
        case .unauthorized:
            title = String(localized: .Onboarding2.onboardingProblemModelUnauthorized)
            nextStep = String(localized: .Onboarding3.onboardingProblemModelUnauthorizedNext)
        case .upstream:
            title = String(localized: .Onboarding1.onboardingProblemModelUpstream)
            nextStep = String(localized: .Onboarding0.onboardingProblemModelUpstreamNext)
        case .protocol:
            title = String(localized: .Onboarding4.onboardingProblemModelProtocol)
            nextStep = String(localized: .Onboarding5.onboardingProblemModelProtocolNext)
        }
        return Problem(title: title, nextStep: nextStep, detail: message)
    }
}

extension StoreProblem {
    var onboardingProblem: Problem {
        let title: String
        switch reason {
        case .unreachable:
            title = String(localized: .Onboarding5.onboardingProblemStoreUnreachable)
        case .timeout:
            title = String(localized: .Onboarding2.onboardingProblemStoreTimeout)
        case .rateLimited:
            title = String(localized: .Onboarding5.onboardingProblemStoreRateLimited)
        case .invalidIndex:
            title = String(localized: .Onboarding3.onboardingProblemStoreInvalidIndex)
        case .indexTooNew:
            title = String(localized: .Onboarding0.onboardingProblemStoreIndexTooNew)
        case .upstream:
            title = String(localized: .Onboarding2.onboardingProblemStoreUpstream)
        case .incomplete:
            title = String(localized: .Onboarding0.onboardingProblemStoreIncomplete)
        case .disabled:
            title = String(localized: .Onboarding4.onboardingProblemStoreDisabled)
        case .other:
            title = String(localized: .Onboarding3.onboardingProblemStoreOther)
        }
        return Problem(title: title, nextStep: String(localized: .Onboarding0.onboardingProblemStoreNext),
            detail: details)
    }
}
