import UIKit
import XCTest

/// Uses the existing no-backend fixture; run on a portrait, home-indicator iPhone in either system appearance.
/// Capture every entry before comparing pixels so a failing baseline still exports the complete walkthrough.
final class SafeAreaUITests: XCTestCase {
    private var rootButtonFrame: CGRect = .zero
    private var app: XCUIApplication!

    override func setUpWithError() throws {
        continueAfterFailure = false
        XCUIDevice.shared.orientation = .portrait
        app = XCUIApplication()
        app.launchArguments += ["-AppleLanguages", "(en)", "-AppleLocale", "en_US"]
        app.launchEnvironment["WONTON_DEV_DATA_SUFFIX"] = UUID().uuidString
        app.launchEnvironment["WONTON_UI_FAKE"] = "1"
        UITestBackend.launch(app)
    }

    override func tearDownWithError() throws {
        app.terminate()
    }

    func testMainEntriesFillSystemSafeAreas() throws {
        var captures: [Capture] = []
        let sidebar = app.buttons.matching(UITestSidebar.button).firstMatch
        XCTAssertTrue(sidebar.waitForExistence(timeout: 10))
        rootButtonFrame = sidebar.frame
        captures.append(try capture("list"))

        sidebar.tap()
        // The scrim covers the shifted main page and only exists while the sidebar is open; its frame is
        // the page frame, sampled after the spring has settled rather than while it is still moving.
        let collapse = app.buttons["Collapse sidebar"].firstMatch
        XCTAssertTrue(collapse.waitForExistence(timeout: 10))
        XCTAssertTrue(collapse.isHittable)
        let settings = app.buttons["Settings"].firstMatch
        XCTAssertTrue(settings.waitForExistence(timeout: 10))
        let settingsButtonFrame = settings.frame
        let pageFrame = try XCTUnwrap(waitForStableFrame(collapse, timeout: 10), app.debugDescription)
        XCTAssertGreaterThan(pageFrame.minX, app.frame.width * 0.6, "shifted page=\(pageFrame)")
        captures.append(try capture("sidebar-open", pageLeft: pageFrame.minX))

        collapse.tap()
        // The scrim only exists while progress > 0, so its disappearance means the page is back at rest.
        XCTAssertTrue(collapse.waitForNonExistence(timeout: 10))
        captures.append(try capture("sidebar-closed"))

        let allApps = try XCTUnwrap(UITestSidebar.allApps(app))
        allApps.tap()
        // The sidebar keeps its own hidden Settings button; wait for one that can actually be tapped.
        // `hittable` is not a valid key path in an element query predicate, so check it on the elements.
        let settingsButtons = app.buttons.matching(NSPredicate(format: "label == %@", "Settings"))
        XCTAssertTrue(waitForHittable(settingsButtons, timeout: 10))
        XCTAssertTrue(collapse.waitForNonExistence(timeout: 10))
        captures.append(try capture("my-apps"))

        // The apps entry is a full-screen cover without a fixture-backed dismissal action.
        // Relaunch the same fixture rather than relying on core navigation mutations it does not implement.
        app.terminate()
        UITestBackend.launch(app)
        XCTAssertTrue(sidebar.waitForExistence(timeout: 10))
        let settingsEntry = try XCTUnwrap(UITestSidebar.row(app, labels: ["Settings"]))
        settingsEntry.tap()
        // The chat list's backup-reminder dismiss button is labelled Close too and stays in the tree
        // behind the cover, so skip it by identifier; the cover slides in, so wait until its button
        // stops moving before capturing.
        let close = app.buttons.matching(NSPredicate(
            format: "label == %@ AND identifier != %@", "Close", "chat-backup-reminder-dismiss"
        )).firstMatch
        XCTAssertTrue(close.waitForExistence(timeout: 10), app.debugDescription)
        XCTAssertNotNil(waitForStableFrame(close, timeout: 10), app.debugDescription)
        captures.append(try capture("settings"))

        // These are layout assertions, not copy assertions. Backgrounds may extend; controls must not.
        XCTAssertGreaterThanOrEqual(rootButtonFrame.minY, 44)
        XCTAssertLessThanOrEqual(settingsButtonFrame.maxY, app.frame.height - 20)
        for captured in captures {
            try assertContinuousBackground(captured)
        }
    }

    private func waitForHittable(_ query: XCUIElementQuery, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        repeat {
            if query.allElementsBoundByIndex.contains { $0.isHittable } { return true }
            RunLoop.current.run(until: Date().addingTimeInterval(0.2))
        } while Date() < deadline
        return false
    }

    /// Waits until the element's frame stops changing and returns it; accessibility frames follow
    /// SwiftUI transitions, so this is the observable "animation finished" condition.
    private func waitForStableFrame(_ element: XCUIElement, timeout: TimeInterval) -> CGRect? {
        let deadline = Date().addingTimeInterval(timeout)
        var last = CGRect.null
        repeat {
            let frame = element.frame
            if !frame.isEmpty, frame == last { return frame }
            last = frame
            RunLoop.current.run(until: Date().addingTimeInterval(0.1))
        } while Date() < deadline
        return nil
    }

    private struct Capture {
        let name: String
        let image: CGImage
        /// Left edge of the shifted main page; only set while the sidebar is open.
        let pageLeft: CGFloat?
    }

    private func capture(_ entry: String, pageLeft: CGFloat? = nil) throws -> Capture {
        // XCUIScreen, not app.screenshot(): status bar and home-indicator pixels are part of the contract.
        let screenshot = XCUIScreen.main.screenshot()
        let image = try XCTUnwrap(screenshot.image.cgImage)
        let name = "safe-area-\(entry)"
        let attachment = XCTAttachment(data: screenshot.pngRepresentation, uniformTypeIdentifier: "public.png")
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        return Capture(name: name, image: image, pageLeft: pageLeft)
    }

    private func assertContinuousBackground(_ captured: Capture) throws {
        // Five-pixel patches in the page margin avoid the clock, island, home indicator and page content:
        // 2% of the width is 8pt, inside the 16pt page inset, so buttons and cards never reach it.
        var columns: [CGFloat] = [0.02, 0.98]
        var edges: [CGFloat] = [0.02, 0.98]
        if let pageLeft = captured.pageLeft {
            // The sidebar covers the screen's left edge and clips the page's right one, so sample the
            // shifted page's own 16pt page inset instead: 3.5pt inside the page edge, patch clear of the
            // 40pt rounded corner. 7% / 93% of the height keep it in the page's top and bottom safe
            // margins, clear of the clock, the island and the home indicator.
            columns = [(pageLeft + 3.5) / app.frame.width]
            edges = [0.07, 0.93]
        }
        for column in columns {
            let middle = try patch(captured.image, x: column, y: 0.5)
            for edge in edges {
                let systemArea = try patch(captured.image, x: column, y: edge)
                assertMatches(systemArea, middle, context: "\(captured.name) x=\(column), y=\(edge)")
            }
        }
    }

    private func assertMatches(_ actual: [UInt8], _ expected: [UInt8], context: String) {
        let differences = zip(actual, expected).map { abs(Int($0) - Int($1)) }
        // The original light-mode band differs by 11 (244 versus 255). Four allows rendering noise
        // without accepting that regression; dark mode uses the same channel threshold.
        XCTAssertLessThanOrEqual(differences.max() ?? 255, 4,
                                 "\(context): background channels \(actual) differ from middle \(expected)")
    }

    private func patch(_ image: CGImage, x normalizedX: CGFloat, y normalizedY: CGFloat) throws -> [UInt8] {
        let side = 5
        let area = CGRect(x: Int(CGFloat(image.width) * normalizedX) - side / 2,
                          y: Int(CGFloat(image.height) * normalizedY) - side / 2,
                          width: side, height: side)
        let cropped = try XCTUnwrap(image.cropping(to: area))
        let space = try XCTUnwrap(CGColorSpace(name: CGColorSpace.sRGB))
        var bytes = [UInt8](repeating: 0, count: side * side * 4)
        try bytes.withUnsafeMutableBytes { buffer in
            let context = try XCTUnwrap(CGContext(
                data: buffer.baseAddress, width: side, height: side, bitsPerComponent: 8,
                bytesPerRow: side * 4, space: space,
                bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue | CGImageAlphaInfo.premultipliedLast.rawValue
            ))
            context.draw(cropped, in: CGRect(x: 0, y: 0, width: side, height: side))
        }
        // Keep every RGB sample: an average could hide a thin seam within a patch.
        return bytes.enumerated().compactMap { index, byte in index % 4 == 3 ? nil : byte }
    }
}
