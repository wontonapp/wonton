import CoreImage.CIFilterBuiltins
import UIKit

/// 用 Core Image 画二维码：每个模块放大成整数个像素，缩放时不会糊。
enum QRImage {
    static func make(_ text: String) -> UIImage? {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        // 深色页面上二维码四周也要有白边，否则扫不出来。
        guard let code = filter.outputImage else { return nil }
        let quiet = code.extent.insetBy(dx: -2, dy: -2)
        let framed = code.composited(over: CIImage(color: .white)).cropped(to: quiet)
        let output = framed.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        guard let image = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return UIImage(cgImage: image)
    }
}
