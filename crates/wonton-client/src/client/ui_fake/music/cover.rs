//! 假曲库的封面：按 ID 挑两种颜色画斜向渐变，再叠一个淡圆，存成 24 位 BMP（iOS、Android 都能直接解）。

/// 同一张专辑每次都是同样的颜色：从这组配色里按 ID 挑。
const PALETTE: [[u8; 3]; 10] = [
    [0xF2, 0x6B, 0x5B],
    [0xF7, 0xB5, 0x38],
    [0x4C, 0xB9, 0x8A],
    [0x3A, 0x86, 0xD9],
    [0x8E, 0x63, 0xD6],
    [0xE0, 0x5E, 0x9C],
    [0x2E, 0x3A, 0x59],
    [0x1F, 0x9E, 0xA8],
    [0xC9, 0x7B, 0x4A],
    [0x5D, 0x6B, 0x7A],
];

/// 边长上限：浏览层最大要 1280，假图不必那么大。
const MAX_SIDE: usize = 640;

fn hash(id: &str) -> usize {
    id.bytes()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
        })
        .try_into()
        .unwrap_or(usize::MAX)
}

fn mix(a: u8, b: u8, t: f32) -> u8 {
    // 两端都在 0..=255 里，插值的结果也在。
    (f32::from(a) + (f32::from(b) - f32::from(a)) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

/// `side` 像素见方的封面。
pub(super) fn bmp(id: &str, side: usize) -> Vec<u8> {
    let side = side.clamp(16, MAX_SIDE);
    let h = hash(id);
    let from = PALETTE[h % PALETTE.len()];
    let to = PALETTE[(h / PALETTE.len() + 3) % PALETTE.len()];
    let row = (side * 3).div_ceil(4) * 4;
    let pixels = row * side;
    let header = 14 + 40;
    let mut out = Vec::with_capacity(header + pixels);
    let u32le = |value: usize| u32::try_from(value).unwrap_or(u32::MAX).to_le_bytes();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&u32le(header + pixels));
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&u32le(header));
    out.extend_from_slice(&u32le(40));
    out.extend_from_slice(&u32le(side));
    out.extend_from_slice(&u32le(side));
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&24_u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    let size = side as f32;
    let (cx, cy, radius) = (size * 0.68, size * 0.34, size * 0.3);
    // BMP 从最下面一行往上存。
    for y in (0..side).rev() {
        let start = out.len();
        for x in 0..side {
            let (fx, fy) = (x as f32, y as f32);
            let t = (fx + fy) / (2.0 * size);
            let glow = if (fx - cx).hypot(fy - cy) < radius {
                0.18
            } else {
                0.0
            };
            for channel in [2, 1, 0] {
                let base = mix(from[channel], to[channel], t);
                out.push(mix(base, 255, glow));
            }
        }
        out.resize(start + row, 0);
    }
    out
}
