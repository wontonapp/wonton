import SwiftUI
import WontonClient

struct OnboardingProfileStepView: View {
    let profile: OnboardingProfileStep
    let send: (Action) -> Void
    @State private var name: String
    @State private var email: String

    init(profile: OnboardingProfileStep, send: @escaping (Action) -> Void) {
        self.profile = profile
        self.send = send
        _name = State(initialValue: profile.displayName)
        _email = State(initialValue: profile.email)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: WontonTheme.spacing.sectionGap) {
            OwnerProfileFields(
                name: Binding(get: { name }, set: {
                    name = $0
                    send(.onboarding(action: .editOnboardingProfile(displayName: name, email: email)))
                }),
                email: Binding(get: { email }, set: {
                    email = $0
                    send(.onboarding(action: .editOnboardingProfile(displayName: name, email: email)))
                }),
                nameProblem: profile.displayNameProblem, emailProblem: profile.emailProblem
            )
        }
    }
}
