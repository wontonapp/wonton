extension TimelineSubscription {
    /// 批次的语义见 `TimelineSubscription`；取消方式同 `ChatSubscription.updates()`。
    public func updates() -> AsyncStream<TimelineUpdate> {
        AsyncStream {
            await withTaskCancellationHandler {
                await self.next()
            } onCancel: {
                self.cancel()
            }
        }
    }
}
