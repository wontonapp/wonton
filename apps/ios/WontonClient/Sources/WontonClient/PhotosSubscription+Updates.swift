extension PhotosSubscription {
    /// 把订阅包装成 `AsyncStream`，用法和取消方式同 `ChatSubscription.updates()`。
    public func updates() -> AsyncStream<PhotosViewModel> {
        AsyncStream {
            await withTaskCancellationHandler {
                await self.next()
            } onCancel: {
                self.cancel()
            }
        }
    }
}
