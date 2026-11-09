extension ChatSubscription {
    /// 把订阅包装成 `AsyncStream`，在 SwiftUI 的 `.task` 里用 `for await` 消费。
    ///
    /// UniFFI 0.32 不会把 Swift Task 的取消传给 Rust，所以这里在取消时调用 `cancel()`，
    /// 让等待中的 `next()` 立即返回 `nil`，流随之结束，Rust 侧的订阅也被释放。
    public func updates() -> AsyncStream<ChatViewModel> {
        AsyncStream {
            await withTaskCancellationHandler {
                await self.next()
            } onCancel: {
                self.cancel()
            }
        }
    }
}
