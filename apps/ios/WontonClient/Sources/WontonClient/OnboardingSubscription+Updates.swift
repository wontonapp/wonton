extension OnboardingSubscription {
    public func updates() -> AsyncStream<OnboardingViewModel> {
        AsyncStream {
            await withTaskCancellationHandler {
                await self.next()
            } onCancel: {
                self.cancel()
            }
        }
    }
}
