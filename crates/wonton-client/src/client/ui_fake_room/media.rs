//! 假房间照片的字节：无压缩的 24 位 BMP。

pub(super) const PHOTO_TOKEN: &str = "ui-fake-photo";
pub(super) const PHOTO_WIDTH: u64 = 320;
pub(super) const PHOTO_HEIGHT: u64 = 240;
/// 四张照片的颜色，BGR 像素序：深蓝、琥珀、绿、紫（紫是单张那幅）。
const PHOTO_COLORS: [[u8; 3]; 4] = [
    [180, 105, 25],
    [25, 165, 190],
    [90, 160, 70],
    [150, 60, 120],
];

/// 假房间图片的原文件：一张无压缩的 24 位 BMP，四张颜色不同（最后一张是单张那幅）。
/// 假数据入口的 [`ClientCore::load_media`] 直接在这里取，不进下载。
pub(crate) fn media(token: &str) -> Option<Vec<u8>> {
    let position: usize = token
        .strip_prefix(PHOTO_TOKEN)?
        .strip_prefix('-')?
        .parse()
        .ok()?;
    PHOTO_COLORS
        .get(position)
        .map(|&bgr| bitmap(PHOTO_WIDTH as u32, PHOTO_HEIGHT as u32, bgr))
}

/// 无压缩的 24 位 BMP：头 + 从下往上的 BGR 像素行，每行补齐到 4 字节。
fn bitmap(width: u32, height: u32, bgr: [u8; 3]) -> Vec<u8> {
    let row = width as usize * 3;
    let stride = row.div_ceil(4) * 4;
    let size = 54 + stride * height as usize;
    let mut out = vec![0u8; size];
    out[0..2].copy_from_slice(b"BM");
    out[2..6].copy_from_slice(&(size as u32).to_le_bytes());
    out[10..14].copy_from_slice(&54u32.to_le_bytes());
    out[14..18].copy_from_slice(&40u32.to_le_bytes());
    out[18..22].copy_from_slice(&width.to_le_bytes());
    out[22..26].copy_from_slice(&height.to_le_bytes());
    out[26..28].copy_from_slice(&1u16.to_le_bytes());
    out[28..30].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..height as usize {
        let base = 54 + y * stride;
        for x in 0..width as usize {
            out[base + x * 3..base + x * 3 + 3].copy_from_slice(&bgr);
        }
    }
    out
}
