import UIKit

/// ThumbHash 占位图的解码（缩略图到之前先画它）。算法是公开的格式说明：亮度和两个色度各一组低频余弦系数，
/// 头五个字节是直流分量和各通道的缩放，其后每个系数四位。
enum ThumbHash {
    @MainActor private static let cache = NSCache<NSString, UIImage>()

    /// base64 的 ThumbHash 解成一张小图；格式不对回空，占位就用底色。
    @MainActor
    static func image(_ base64: String) -> UIImage? {
        if let hit = cache.object(forKey: base64 as NSString) { return hit }
        guard let data = Data(base64Encoded: base64), let image = decode([UInt8](data)) else { return nil }
        cache.setObject(image, forKey: base64 as NSString)
        return image
    }

    private struct Header {
        let dcL: Double, dcP: Double, dcQ: Double
        let scaleL: Double, scaleP: Double, scaleQ: Double
        let hasAlpha: Bool
        let countX: Int, countY: Int

        init?(_ hash: [UInt8]) {
            guard hash.count >= 5 else { return nil }
            let low = Int(hash[0]) | Int(hash[1]) << 8 | Int(hash[2]) << 16
            let high = Int(hash[3]) | Int(hash[4]) << 8
            dcL = Double(low & 63) / 63
            dcP = Double((low >> 6) & 63) / 31.5 - 1
            dcQ = Double((low >> 12) & 63) / 31.5 - 1
            scaleL = Double((low >> 18) & 31) / 31
            hasAlpha = (low >> 23) != 0
            scaleP = Double((high >> 3) & 63) / 63
            scaleQ = Double((high >> 9) & 63) / 63
            let landscape = (high >> 15) != 0
            let long = hasAlpha ? 5 : 7
            countX = max(3, landscape ? long : high & 7)
            countY = max(3, landscape ? high & 7 : long)
        }
    }

    /// 按顺序取四位一个的交流系数。
    private struct Coefficients {
        let hash: [UInt8]
        var index = 0
        let start: Int

        /// 一个通道的系数，顺序同 `terms(countX, countY)`。
        mutating func channel(_ countX: Int, _ countY: Int, scale: Double) -> [Double]? {
            var values: [Double] = []
            for _ in ThumbHash.terms(countX, countY) {
                let byte = start + (index >> 1)
                guard byte < hash.count else { return nil }
                let nibble = (Int(hash[byte]) >> ((index & 1) << 2)) & 15
                values.append((Double(nibble) / 7.5 - 1) * scale)
                index += 1
            }
            return values
        }
    }

    /// 一个通道留下的余弦项（横向第几个、纵向第几个）：左上角的三角，直流那一项不在里面。
    private static func terms(_ countX: Int, _ countY: Int) -> [(column: Int, row: Int)] {
        var list: [(column: Int, row: Int)] = []
        for row in 0 ..< countY {
            var column = row == 0 ? 1 : 0
            while column * countY < countX * (countY - row) {
                list.append((column, row))
                column += 1
            }
        }
        return list
    }

    /// 每个像素位置上各阶余弦的值。
    private static func waves(_ length: Int, _ count: Int) -> [[Double]] {
        (0 ..< length).map { position in
            (0 ..< count).map { cos(Double.pi / Double(length) * (Double(position) + 0.5) * Double($0)) }
        }
    }

    private static func decode(_ hash: [UInt8]) -> UIImage? {
        guard let header = Header(hash) else { return nil }
        var coefficients = Coefficients(hash: hash, start: header.hasAlpha ? 6 : 5)
        guard let lumaValues = coefficients.channel(header.countX, header.countY, scale: header.scaleL),
              let chromaP = coefficients.channel(3, 3, scale: header.scaleP * 1.25),
              let chromaQ = coefficients.channel(3, 3, scale: header.scaleQ * 1.25)
        else { return nil }
        let ratio = Double(header.countX) / Double(header.countY)
        let width = Int((ratio > 1 ? 32 : 32 * ratio).rounded())
        let height = Int((ratio > 1 ? 32 / ratio : 32).rounded())
        guard width > 0, height > 0 else { return nil }
        let lumaTerms = terms(header.countX, header.countY)
        let chromaTerms = terms(3, 3)
        let across = waves(width, max(header.countX, 3))
        let down = waves(height, max(header.countY, 3))
        var pixels = [UInt8](repeating: 255, count: width * height * 4)
        for row in 0 ..< height {
            for column in 0 ..< width {
                let wave = { (term: (column: Int, row: Int)) in across[column][term.column] * down[row][term.row] * 2 }
                var luma = header.dcL, colorP = header.dcP, colorQ = header.dcQ
                for (index, term) in lumaTerms.enumerated() { luma += lumaValues[index] * wave(term) }
                for (index, term) in chromaTerms.enumerated() {
                    colorP += chromaP[index] * wave(term)
                    colorQ += chromaQ[index] * wave(term)
                }
                let blue = luma - 2 / 3 * colorP
                let red = (3 * luma - blue + colorQ) / 2
                let green = red - colorQ
                let offset = (row * width + column) * 4
                for (channel, value) in [red, green, blue].enumerated() {
                    pixels[offset + channel] = UInt8(max(0, min(1, value)) * 255)
                }
            }
        }
        return bitmap(pixels, width: width, height: height)
    }

    private static func bitmap(_ pixels: [UInt8], width: Int, height: Int) -> UIImage? {
        guard let provider = CGDataProvider(data: Data(pixels) as CFData),
              let image = CGImage(
                  width: width, height: height, bitsPerComponent: 8, bitsPerPixel: 32, bytesPerRow: width * 4,
                  space: CGColorSpaceCreateDeviceRGB(),
                  bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.noneSkipLast.rawValue),
                  provider: provider, decode: nil, shouldInterpolate: true, intent: .defaultIntent
              )
        else { return nil }
        return UIImage(cgImage: image)
    }
}
