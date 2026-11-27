//! 假相册的图：按名字定色的一张风景（天、远山、地、一轮太阳），24 位 BMP。只为让网格、大图和缩放有东西可看，
//! 同一个名字每次画出来一样。

use std::f64::consts::PI;

/// 名字散成一个数（FNV-1a），颜色和形状都从它来。
fn seed(name: &str) -> u64 {
    name.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// 色相（度）、饱和度、明度转成 RGB，各 0–1。
fn hsv(hue: f64, saturation: f64, value: f64) -> [f64; 3] {
    let sector = hue.rem_euclid(360.0) / 60.0;
    let chroma = value * saturation;
    let second = chroma * (1.0 - (sector % 2.0 - 1.0).abs());
    let (r, g, b) = match sector as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let lift = value - chroma;
    [r + lift, g + lift, b + lift]
}

fn mix(from: [f64; 3], to: [f64; 3], amount: f64) -> [f64; 3] {
    let amount = amount.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| from[i] + (to[i] - from[i]) * amount)
}

struct Scene {
    sky_top: [f64; 3],
    sky_low: [f64; 3],
    hill: [f64; 3],
    ground: [f64; 3],
    sun: (f64, f64),
    phase: f64,
}

impl Scene {
    fn new(name: &str) -> Self {
        let seed = seed(name);
        let hue = (seed % 360) as f64;
        let unit = |shift: u32| ((seed >> shift) % 1000) as f64 / 1000.0;
        Self {
            sky_top: hsv(hue, 0.55, 0.92),
            sky_low: hsv(hue + 35.0, 0.25, 1.0),
            hill: hsv(hue + 170.0, 0.35, 0.55),
            ground: hsv(hue + 140.0, 0.45, 0.38),
            sun: (0.2 + unit(12) * 0.6, 0.18 + unit(24) * 0.2),
            phase: unit(36) * 2.0 * PI,
        }
    }

    /// 一个点的颜色；`x`、`y` 是 0–1，`y` 从上往下，`ratio` 是宽比高。
    fn color(&self, x: f64, y: f64, ratio: f64) -> [f64; 3] {
        let ridge =
            0.58 + 0.07 * (x * 5.0 + self.phase).sin() + 0.03 * (x * 13.0 + self.phase).cos();
        if y > 0.74 {
            return mix(self.ground, self.hill, (0.9 - y) * 2.0);
        }
        if y > ridge {
            return mix(self.hill, self.ground, (y - ridge) / (0.74 - ridge));
        }
        let sky = mix(self.sky_top, self.sky_low, y / ridge);
        let distance = ((x - self.sun.0) * ratio).hypot(y - self.sun.1);
        mix([1.0, 0.97, 0.86], sky, (distance - 0.06) / 0.1)
    }
}

/// 画一张长边 `long_side` 像素、宽比高为 `aspect` 的图。
pub(super) fn bmp(name: &str, long_side: u32, aspect: f64) -> Vec<u8> {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect.clamp(0.25, 4.0)
    } else {
        1.0
    };
    let short = |ratio: f64| ((f64::from(long_side) * ratio).round() as u32).max(1);
    let (width, height) = if aspect >= 1.0 {
        (long_side, short(1.0 / aspect))
    } else {
        (short(aspect), long_side)
    };
    let scene = Scene::new(name);
    // 每行补到 4 字节的整数倍；行从下往上存，颜色按蓝绿红排。
    let row = (width as usize * 3).div_ceil(4) * 4;
    let pixels = row * height as usize;
    let mut out = Vec::with_capacity(54 + pixels);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + pixels as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(pixels as u32).to_le_bytes());
    out.extend_from_slice(&[0; 16]);
    for line in (0..height).rev() {
        let y = (f64::from(line) + 0.5) / f64::from(height);
        for column in 0..width {
            let x = (f64::from(column) + 0.5) / f64::from(width);
            let [r, g, b] = scene.color(x, y, aspect);
            for channel in [b, g, r] {
                out.push((channel.clamp(0.0, 1.0) * 255.0).round() as u8);
            }
        }
        out.resize(out.len() + row - width as usize * 3, 0);
    }
    out
}
