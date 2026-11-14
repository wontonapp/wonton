import Observation
import WontonClient

@MainActor
@Observable
final class ContentPageModel {
    private(set) var page: TabPage?
    private(set) var problem: Problem?
    private let core: ClientCore
    let tab: TabRef
    let webViews = TabWebViewStore()
    private var subscription: ContentPageSubscription?
    var attempt = 0

    init(core: ClientCore, tab: TabRef) {
        self.core = core
        self.tab = tab
    }

    func observe() async {
        page = nil
        problem = nil
        let subscription: ContentPageSubscription
        do {
            subscription = try core.subscribeContentPage(tab: tab)
        } catch {
            problem = error.problem
            return
        }
        self.subscription = subscription
        page = subscription.page()
        await withTaskCancellationHandler {
            while let value = await subscription.next() {
                guard !Task.isCancelled else { break }
                page = value
            }
        } onCancel: {
            subscription.cancel()
        }
        subscription.cancel()
        if self.subscription === subscription { self.subscription = nil }
    }

    func close() {
        subscription?.cancel()
        subscription = nil
        webViews.releaseAll()
        page = nil
    }
}
