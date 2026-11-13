import CoreSpotlight
import Foundation
import OSLog
import UIKit
import UniformTypeIdentifiers
import WidgetKit
import WontonClient

/// One app-lifetime consumer keeps Spotlight replacement and account clearing strictly ordered.
@MainActor
final class SystemEntryModel {
    private static let domain = "app.wonton.system-entry"
    private static let logger = Logger(subsystem: Bundle.main.bundleIdentifier ?? "app.wonton", category: "SystemEntry")
    private var observation: Task<Void, Never>?

    init(core: ClientCore, dataDirectory: URL) {
        let subscription = core.subscribeWidgetSnapshot()
        observation = Task {
            await Self.observe(subscription, dataDirectory: dataDirectory)
        }
    }

    deinit {
        observation?.cancel()
    }

    private static func observe(_ subscription: WidgetSnapshotSubscription, dataDirectory: URL) async {
        await withTaskCancellationHandler {
            // Subscribe before reading so a write during initial indexing cannot be missed.
            switch readWidgetSnapshot(dataDir: dataDirectory.path(percentEncoded: false)) {
            case let .ready(snapshot):
                await replaceIndex(snapshot)
            case .missing:
                await replaceIndex(nil)
            case let .unreadable(detail):
                logger.error("Cannot read initial widget snapshot: \(detail, privacy: .public)")
                await replaceIndex(nil)
            }
            while !Task.isCancelled, let snapshot = await subscription.next() {
                WidgetCenter.shared.reloadAllTimelines()
                await replaceIndex(snapshot)
            }
        } onCancel: {
            subscription.cancel()
        }
    }

    private static func replaceIndex(_ snapshot: WidgetSnapshot?) async {
        let index = CSSearchableIndex.default()
        do {
            // Await deletion and insertion in the same consumer: an older insertion must never
            // complete after a signed-out snapshot has cleared this domain.
            try await index.deleteSearchableItems(withDomainIdentifiers: [domain])
            guard let snapshot, snapshot.account == .signedIn else { return }
            let items = searchableItems(snapshot)
            if !items.isEmpty { try await index.indexSearchableItems(items) }
        } catch {
            logger.error("Cannot replace Spotlight entries: \(String(describing: error), privacy: .public)")
        }
    }

    private static func searchableItems(_ snapshot: WidgetSnapshot) -> [CSSearchableItem] {
        var identifiers = Set<String>()
        var items: [CSSearchableItem] = []
        func append(name: String, link: AppLink, symbol: String) {
            let identifier = appLinkUrl(link: link)
            guard identifiers.insert(identifier).inserted else { return }
            let attributes = CSSearchableItemAttributeSet(contentType: UTType.content)
            attributes.title = name
            attributes.thumbnailData = UIImage(systemName: symbol)?.pngData()
            items.append(CSSearchableItem(
                uniqueIdentifier: identifier, domainIdentifier: domain, attributeSet: attributes
            ))
        }
        for room in snapshot.rooms {
            append(name: room.name, link: .room(roomId: room.roomId), symbol: room.isDirect ? "person.crop.circle" : "bubble.left.and.bubble.right")
        }
        for contact in snapshot.contacts {
            append(name: contact.name, link: .room(roomId: contact.roomId), symbol: contact.isBot ? "sparkles" : "person.crop.circle")
        }
        for plugin in snapshot.plugins {
            append(name: plugin.title, link: .tab(tab: .plugin(pluginId: plugin.pluginId, page: nil)), symbol: "square.grid.2x2")
        }
        return items
    }
}
