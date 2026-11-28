import WontonClient

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
