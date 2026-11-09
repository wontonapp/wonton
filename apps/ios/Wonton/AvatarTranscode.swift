import AVFoundation
import CoreGraphics
import ImageIO
import PhotosUI
import SwiftUI
import UIKit
import UniformTypeIdentifiers
import WontonClient

/// 选取到 `AvatarUpload` 的转码（ADR-0044：核心不转码，上限从核心拿，不写死）。
enum AvatarTranscode {
    // 转码管线的目标值是规格定的（ADR-0044），不是上限：上限 maxAvatarEdge/Bytes/DurationMs 从核心读。
    private static let imageTargetEdge: CGFloat = 512
    private static let videoTargetEdge: CGFloat = 480
    private static let videoStartBitrate = 1_500_000
    /// 超过上限就按比例降码率重转，最多三轮；仍超才报错。
    private static let bitrateRetries = 3

    static func upload(
        from item: PhotosPickerItem,
        slot: AvatarSlot,
        core: ClientCore,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws -> AvatarUpload {
        if item.isMovie {
            return try await transcodeVideo(item: item, core: core, progress: progress)
        }
        return try await transcodeImage(item: item, slot: slot, core: core, progress: progress)
    }

    // MARK: 图片

    private static func transcodeImage(
        item: PhotosPickerItem,
        slot: AvatarSlot,
        core: ClientCore,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws -> AvatarUpload {
        guard let data = try await item.loadTransferable(type: Data.self), !data.isEmpty else {
            throw AvatarTranscodeError.unreadable
        }
        if let animated = try animatedUpload(data: data, item: item, slot: slot, core: core) {
            progress(1)
            return animated
        }
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let cgImage = CGImageSourceCreateImageAtIndex(source, 0, nil) else {
            throw AvatarTranscodeError.unreadable
        }
        // PNG 有透明才输出 PNG，其余输出 JPEG（规格：质量 0.85）。
        let isPNG = (CGImageSourceGetType(source) as String?) == UTType.png.identifier
        let keepPNG = isPNG && hasAlpha(source)
        progress(0.4)
        let image = renderSquare(UIImage(cgImage: cgImage), edge: imageTargetEdge, opaque: !keepPNG)
        let bytes = keepPNG ? image.pngData() : image.jpegData(compressionQuality: 0.85)
        guard let bytes, !bytes.isEmpty else { throw AvatarTranscodeError.unreadable }
        progress(1)
        return AvatarUpload(mimeType: keepPNG ? "image/png" : "image/jpeg", data: bytes, poster: nil)
    }

    /// 动图（GIF、动图 WebP）合规就原样交；返回 nil 表示不是动图（ADR-0044）。
    private static func animatedUpload(
        data: Data,
        item: PhotosPickerItem,
        slot: AvatarSlot,
        core: ClientCore
    ) throws -> AvatarUpload? {
        let frames = CGImageSourceCreateWithData(data as CFData, nil).map { CGImageSourceGetCount($0) } ?? 0
        guard item.isGIF || frames > 1 else { return nil }
        let (square, edge) = squareEdge(data)
        if square, edge <= Int(core.maxAvatarEdge()), data.count <= Int(core.maxAvatarBytes()),
           let mime = item.supportedContentTypes.first?.preferredMIMEType {
            return AvatarUpload(mimeType: mime, data: data, poster: nil)
        }
        if slot.allowsVideo {
            throw AvatarTranscodeError.needsVideo(maxDurationMs: core.maxAvatarDurationMs())
        }
        if !square { throw AvatarTranscodeError.notSquare }
        if edge > Int(core.maxAvatarEdge()) { throw AvatarTranscodeError.tooBig }
        throw AvatarTranscodeError.tooLarge
    }

    private static func squareEdge(_ data: Data) -> (square: Bool, edge: Int) {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let cgImage = CGImageSourceCreateImageAtIndex(source, 0, nil) else { return (false, 0) }
        return (cgImage.width == cgImage.height, max(cgImage.width, cgImage.height))
    }

    private static func hasAlpha(_ source: CGImageSource) -> Bool {
        let props = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any]
        return (props?[kCGImagePropertyHasAlpha] as? Bool) ?? false
    }

    /// 居中裁成正方形再缩放；`UIImage.draw` 自己处理 EXIF 取向，负起点把裁剪框对到中心。
    private static func renderSquare(_ source: UIImage, edge: CGFloat, opaque: Bool) -> UIImage {
        let side = min(source.size.width, source.size.height)
        let scale = edge / side
        let format = UIGraphicsImageRendererFormat()
        format.scale = 1
        format.opaque = opaque
        return UIGraphicsImageRenderer(size: CGSize(width: edge, height: edge), format: format).image { _ in
            source.draw(in: CGRect(
                x: -(source.size.width - side) / 2 * scale,
                y: -(source.size.height - side) / 2 * scale,
                width: source.size.width * scale,
                height: source.size.height * scale
            ))
        }
    }

    // MARK: 视频

    private static func transcodeVideo(
        item: PhotosPickerItem,
        core: ClientCore,
        progress: @escaping @Sendable (Double) -> Void
    ) async throws -> AvatarUpload {
        guard let movie = try await item.loadTransferable(type: PickedMovie.self) else {
            throw AvatarTranscodeError.unreadable
        }
        defer { try? FileManager.default.removeItem(at: movie.url) }
        let asset = AVURLAsset(url: movie.url)
        let full = try await asset.load(.duration).seconds
        let maxSeconds = Double(core.maxAvatarDurationMs()) / 1000
        let duration = CMTime(seconds: min(full > 0 ? full : 0, maxSeconds), preferredTimescale: 600)
        guard duration.seconds > 0 else { throw AvatarTranscodeError.unreadable }

        var bitrate = videoStartBitrate
        for attempt in 0..<bitrateRetries {
            let base = Double(attempt) / Double(bitrateRetries)
            let url = try await writeVideo(asset: asset, duration: duration, bitrate: bitrate) { inside in
                progress(base + inside / Double(bitrateRetries))
            }
            let size = (try? FileManager.default.attributesOfItem(atPath: url.path))?[.size] as? Int
            if let size, size <= Int(core.maxAvatarBytes()) {
                progress(0.9)
                let poster = try await extractPoster(url: url)
                let data = try Data(contentsOf: url)
                try? FileManager.default.removeItem(at: url)
                progress(1)
                return AvatarUpload(mimeType: "video/mp4", data: data, poster: poster)
            }
            try? FileManager.default.removeItem(at: url)
            guard let size else { throw AvatarTranscodeError.unreadable }
            bitrate = max(1, Int(Double(bitrate) * Double(core.maxAvatarBytes()) / Double(size) * 0.95))
        }
        throw AvatarTranscodeError.tooLarge
    }

    /// 读一帧写一帧的 H.264 转码：居中裁方、缩到 480、截到 duration 为止、不写声音轨道。
    private static func writeVideo(
        asset: AVURLAsset,
        duration: CMTime,
        bitrate: Int,
        onProgress: @escaping @Sendable (Double) -> Void
    ) async throws -> URL {
        guard let track = try await asset.loadTracks(withMediaType: .video).first else {
            throw AvatarTranscodeError.unreadable
        }
        let natural = try await track.load(.naturalSize)
        let transform = try await track.load(.preferredTransform)
        let fps = try await track.load(.nominalFrameRate)

        let reader = try AVAssetReader(asset: asset)
        let readerOutput = AVAssetReaderVideoCompositionOutput(
            videoTracks: [track],
            videoSettings: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA]
        )
        readerOutput.videoComposition = makeComposition(track: track, natural: natural, transform: transform)
        guard reader.canAdd(readerOutput) else { throw AvatarTranscodeError.unreadable }
        reader.add(readerOutput)

        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("wonton-avatar-\(UUID().uuidString).mp4")
        let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
        let input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: videoTargetEdge,
            AVVideoHeightKey: videoTargetEdge,
            AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: bitrate]
        ])
        input.expectsMediaDataInRealTime = false
        guard writer.canAdd(input) else { throw AvatarTranscodeError.unreadable }
        writer.add(input)

        guard reader.startReading() else { throw AvatarTranscodeError.unreadable }
        writer.startWriting()

        await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
            VideoPump(
                input: input, reader: reader, output: readerOutput, writer: writer
            ).run(
                duration: duration, expected: max(1, Int(fps * Float(duration.seconds))),
                onProgress: onProgress
            ) {
                continuation.resume()
            }
        }
        guard writer.status == .completed else { throw AvatarTranscodeError.unreadable }
        return url
    }

    /// 逐帧喂给编码器的管线：超过 duration 截止，读完或写失败就收尾，完成时回调一次。
    private struct VideoPump {
        let input: AVAssetWriterInput
        let reader: AVAssetReader
        let output: AVAssetReaderVideoCompositionOutput
        let writer: AVAssetWriter

        func run(
            duration: CMTime,
            expected: Int,
            onProgress: @escaping @Sendable (Double) -> Void,
            done: @escaping () -> Void
        ) {
            var started = false
            var appended = 0
            let finish = {
                input.markAsFinished()
                reader.cancelReading()
                writer.finishWriting { done() }
            }
            input.requestMediaDataWhenReady(on: DispatchQueue(label: "app.wonton.avatar.encode")) {
                while input.isReadyForMoreMediaData {
                    guard let sample = output.copyNextSampleBuffer() else { finish(); return }
                    if !started {
                        writer.startSession(atSourceTime: CMSampleBufferGetPresentationTimeStamp(sample))
                        started = true
                    }
                    if CMSampleBufferGetPresentationTimeStamp(sample) > duration { finish(); return }
                    if input.append(sample) {
                        appended += 1
                        if appended % 15 == 0 {
                            onProgress(min(1, Double(appended) / Double(expected)))
                        }
                    } else {
                        finish()
                        return
                    }
                }
            }
        }
    }

    /// 先按 preferredTransform 摆正，再对准中心裁出正方形缩到 edge（concatenating 是后乘，写在后面的先作用于点）。
    private static func makeComposition(
        track: AVAssetTrack,
        natural: CGSize,
        transform: CGAffineTransform
    ) -> AVMutableVideoComposition {
        let oriented = CGRect(origin: .zero, size: natural).applying(transform)
        let width = abs(oriented.width)
        let height = abs(oriented.height)
        let side = min(width, height)
        var final = transform.translatedBy(x: -oriented.minX, y: -oriented.minY)
        final = CGAffineTransform(translationX: -(width - side) / 2, y: -(height - side) / 2).concatenating(final)
        final = CGAffineTransform(scaleX: videoTargetEdge / side, y: videoTargetEdge / side).concatenating(final)

        let composition = AVMutableVideoComposition()
        composition.renderSize = CGSize(width: videoTargetEdge, height: videoTargetEdge)
        composition.frameDuration = CMTime(value: 1, timescale: 30)
        let layer = AVMutableVideoCompositionLayerInstruction(assetTrack: track)
        layer.setTransform(final, at: .zero)
        let instruction = AVMutableVideoCompositionInstruction()
        instruction.timeRange = CMTimeRange(start: .zero, duration: CMTime(value: .max, timescale: 1))
        instruction.layerInstructions = [layer]
        composition.instructions = [instruction]
        return composition
    }

    private static func extractPoster(url: URL) async throws -> Data? {
        let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
        generator.appliesPreferredTrackTransform = true
        generator.requestedTimeToleranceBefore = .zero
        generator.requestedTimeToleranceAfter = CMTime(seconds: 0.5, preferredTimescale: 600)
        return await withCheckedContinuation { continuation in
            generator.generateCGImagesAsynchronously(forTimes: [NSValue(time: .zero)]) { _, cgImage, _, _, _ in
                if let cgImage, let data = UIImage(cgImage: cgImage).jpegData(compressionQuality: 0.8) {
                    continuation.resume(returning: data)
                } else {
                    continuation.resume(returning: nil)
                }
            }
        }
    }
}

/// 转码失败的原因，按「文件不行」的具体条目显示（ADR-0044）。
enum AvatarTranscodeError: Error {
    case unreadable
    case tooLarge
    case tooBig
    case notSquare
    /// 动图不合规：状态头像提示换用视频，上限是核心的 maxAvatarDurationMs。
    case needsVideo(maxDurationMs: UInt32)

    var problem: Problem {
        switch self {
        case .unreadable:
            Problem(title: String(localized: .botProfileProblemMediaUnreadable))
        case .tooLarge:
            Problem(title: String(localized: .botProfileProblemMediaTooLarge))
        case .tooBig:
            Problem(title: String(localized: .botProfileProblemMediaTooBig))
        case .notSquare:
            Problem(title: String(localized: .botProfileProblemMediaNotSquare))
        case let .needsVideo(maxDurationMs):
            Problem(
                title: String(localized: .botProfileProblemMediaTooLarge),
                nextStep: String(localized: .botProfilePickUseVideo(
                    Duration.seconds(Int(maxDurationMs / 1000))
                        .formatted(.units(allowed: [.seconds], width: .wide))
                ))
            )
        }
    }
}
