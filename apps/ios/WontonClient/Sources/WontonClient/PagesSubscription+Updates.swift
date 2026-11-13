extension PagesSubscription {
    public func updates() -> AsyncStream<PagesViewModel> {
        AsyncStream {
            await withTaskCancellationHandler {
                await self.next()
            } onCancel: {
                self.cancel()
            }
        }
    }
}
