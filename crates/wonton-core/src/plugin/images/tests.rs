use image::{Rgba, RgbaImage};

use super::*;

fn png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
    encode_png(RgbaImage::from_pixel(width, height, Rgba(color))).unwrap()
}

fn decoded(data: &[u8]) -> DynamicImage {
    image::load_from_memory(data).unwrap()
}

const RED_SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
  <rect width="24" height="24" fill="#ff0000"/>
</svg>"##;

#[test]
fn png_icons_are_scaled_to_both_sizes() {
    let icon = icon(IconFormat::Png, &png(256, 256, [0, 128, 255, 255])).unwrap();
    for (data, edge) in [(&icon.small, ICON_SMALL), (&icon.large, ICON_LARGE)] {
        assert!(data.starts_with(b"\x89PNG"));
        let image = decoded(data);
        assert_eq!((image.width(), image.height()), (edge, edge));
        assert_eq!(
            image.to_rgba8().get_pixel(edge / 2, edge / 2).0,
            [0, 128, 255, 255]
        );
    }
}

#[test]
fn png_icons_must_be_square_large_enough_and_really_png() {
    let err = icon(IconFormat::Png, &png(300, 200, [0; 4])).unwrap_err();
    assert!(err.contains("正方形"), "{err}");
    let err = icon(IconFormat::Png, &png(128, 128, [0; 4])).unwrap_err();
    assert!(err.contains("至少 256"), "{err}");
    let err = icon(IconFormat::Png, RED_SQUARE.as_bytes()).unwrap_err();
    assert!(err.contains("不是 PNG"), "{err}");
    // 解码前就按头部的尺寸拒绝。
    let err = icon(IconFormat::Png, &png(4097, 1, [0; 4])).unwrap_err();
    assert!(err.contains("4096"), "{err}");
}

#[test]
fn svg_icons_render_without_text_or_external_images() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 24 24">
  <rect width="24" height="24" fill="#ff0000"/>
  <image href="/etc/passwd" width="24" height="24"/>
  <image xlink:href="data:image/png;base64,iVBORw0KGgo=" width="24" height="24"/>
  <text x="2" y="20" font-size="20" fill="#0000ff">W</text>
</svg>"##;
    let pixels = icon_pixels(IconFormat::Svg, svg.as_bytes()).unwrap();
    assert_eq!(pixels.dimensions(), (ICON_LARGE, ICON_LARGE));
    // 文字和图片都没画上去，整张是红的。
    assert!(pixels.pixels().all(|p| p.0 == [255, 0, 0, 255]));
    assert!(!has_transparent_corner(&pixels));
}

#[test]
fn svg_icons_must_be_square_well_formed_svg() {
    let cases = [
        (
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"/>"#,
            "正方形",
        ),
        (r#"<svg viewBox="0 0 10 10"/>"#, "根元素"),
        (r#"<html xmlns="http://www.w3.org/2000/svg"/>"#, "根元素"),
        ("<svg", "不是合法的 SVG"),
        (
            r#"<!DOCTYPE svg [<!ENTITY a "aaaa">]><svg xmlns="http://www.w3.org/2000/svg">&a;</svg>"#,
            "不是合法的 SVG",
        ),
    ];
    for (svg, expected) in cases {
        let err = icon(IconFormat::Svg, svg.as_bytes()).unwrap_err();
        assert!(err.contains(expected), "{svg}：{err}");
    }
    let gzip = [0x1f, 0x8b, 0x08, 0x00];
    assert!(icon(IconFormat::Svg, &gzip).unwrap_err().contains("UTF-8"));
}

/// 每一层引用上一层十次：几百字节展开成一百万个元素。`id` 是各层 id 属性的名字，`link(n)` 是引用第 n 层的属性。
fn use_bomb(id: &str, link: impl Fn(usize) -> String) -> String {
    let mut svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:e="urn:e" viewBox="0 0 1 1"><defs><rect {id}="l0" width="1" height="1"/>"#
    );
    for level in 1..=6 {
        svg.push_str(&format!("<g {id}=\"l{level}\">"));
        for _ in 0..10 {
            svg.push_str(&format!("<use {}/>", link(level - 1)));
        }
        svg.push_str("</g>");
    }
    svg.push_str(r##"</defs><use href="#l6"/></svg>"##);
    svg
}

/// 在交给 resvg 之前就被拒绝。
#[test]
fn use_bombs_and_use_cycles_are_refused() {
    let svg = use_bomb("id", |n| format!("xlink:href=\"#l{n}\""));
    assert!(svg.len() < 2048);
    let err = icon(IconFormat::Svg, svg.as_bytes()).unwrap_err();
    assert!(err.contains("超过 20000 个元素"), "{err}");

    let cycle = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><g id="a"><use href="#a"/></g></svg>"##;
    let err = icon(IconFormat::Svg, cycle.as_bytes()).unwrap_err();
    assert!(err.contains("循环引用"), "{err}");
}

/// 预检不追求和 usvg 认得一样，只认一种写法。评审时的三种绕过：同时写 `href` 和 `xlink:href`（usvg 展开 `href`，旧的预检
/// 数 `xlink:href`）；别的命名空间里的 `id`（usvg 也认）；id 里有制表符（svgtypes 只在空格处断开 id）。
#[test]
fn use_references_have_exactly_one_form() {
    for id in ["xml:id", "e:id"] {
        let svg = use_bomb(id, |n| format!("href=\"#l{n}\""));
        let err = icon(IconFormat::Svg, svg.as_bytes()).unwrap_err();
        assert!(err.contains("超过 20000 个元素"), "{id}：{err}");
    }
    let two = use_bomb("id", |n| format!("xlink:href=\"#l0\" href=\"#l{n}\""));
    let err = icon(IconFormat::Svg, two.as_bytes()).unwrap_err();
    assert!(err.contains("只有一个 href"), "{err}");

    let svg = |attrs: &str| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 1 1"><rect id="a" width="1" height="1"/><rect id="t&#9;0" width="1" height="1"/><use {attrs}/></svg>"#
        )
    };
    let refused = [
        "",
        r##"href="#a" xlink:href="#a""##,
        r##"href="#t&#9;0""##,
        r##"href=" #a""##,
        r##"href="#""##,
        r##"href="icons.svg#a""##,
    ];
    for attrs in refused {
        let err = icon(IconFormat::Svg, svg(attrs).as_bytes()).unwrap_err();
        assert!(err.contains("<use>"), "{attrs}：{err}");
    }
    for attrs in [r##"href="#a""##, r##"xlink:href="#a""##] {
        icon(IconFormat::Svg, svg(attrs).as_bytes()).unwrap();
    }
}

/// 样式表只收不带组合符的选择器：simplecss 匹配后代选择器时逐层回溯，评审时几百字节的图标 20 秒还没算完。
#[test]
fn style_sheets_only_take_selectors_without_combinators() {
    let bomb = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><style>q{}{{fill:red}}</style>{}<rect width="16" height="16"/>{}</svg>"#,
        " g".repeat(24),
        "<g>".repeat(60),
        "</g>".repeat(60)
    );
    assert!(bomb.len() < 1024);
    let err = icon(IconFormat::Svg, bomb.as_bytes()).unwrap_err();
    assert!(err.contains("组合符"), "{err}");

    let svg = |style: &str| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:x="urn:x" viewBox="0 0 24 24">{style}<rect class="a" width="24" height="24"/></svg>"#
        )
    };
    let refused = [
        "<style>g > rect{fill:red}</style>",
        "<style>rect + rect{fill:red}</style>",
        "<style>.b, svg .a{fill:red}</style>",
        // usvg 按本地名找样式表，`type` 不写或是 text/css 的都用。
        r#"<style type="text/css">svg .a{fill:red}</style>"#,
        "<x:style>svg .a{fill:red}</x:style>",
    ];
    for style in refused {
        let err = icon(IconFormat::Svg, svg(style).as_bytes()).unwrap_err();
        assert!(err.contains("组合符"), "{style}：{err}");
    }
    let many: String = (0..257).map(|i| format!(".c{i}{{fill:red}}")).collect();
    let err = icon(
        IconFormat::Svg,
        svg(&format!("<style>{many}</style>")).as_bytes(),
    )
    .unwrap_err();
    assert!(err.contains("最多 256 条"), "{err}");

    // usvg 不用的样式表不管；设计工具导出的写法照常渲染。
    icon(
        IconFormat::Svg,
        svg(r#"<style type="text/x-other">svg .a{fill:red}</style>"#).as_bytes(),
    )
    .unwrap();
    let exported = svg("<style>rect.a, #b {fill:#00f}</style>");
    let pixels = icon_pixels(IconFormat::Svg, exported.as_bytes()).unwrap();
    assert_eq!(pixels.get_pixel(256, 256).0, [0, 0, 255, 255]);
}

/// 层数按展开后算：一串互相引用的 `<use>` 在文件里只有两层。
#[test]
fn deep_nesting_is_refused_after_expanding_use() {
    let nested = |levels: usize| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1">{}{}</svg>"#,
            "<g>".repeat(levels),
            "</g>".repeat(levels)
        )
    };
    icon(IconFormat::Svg, nested(63).as_bytes()).unwrap();
    let err = icon(IconFormat::Svg, nested(64).as_bytes()).unwrap_err();
    assert!(err.contains("嵌套超过 64 层"), "{err}");

    let mut chain = String::from(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><defs><rect id="u0" width="1" height="1"/>"#,
    );
    for i in 1..=70 {
        chain.push_str(&format!(r##"<use id="u{i}" href="#u{}"/>"##, i - 1));
    }
    chain.push_str(r##"</defs><use href="#u70"/></svg>"##);
    let err = icon(IconFormat::Svg, chain.as_bytes()).unwrap_err();
    assert!(err.contains("嵌套超过 64 层"), "{err}");
}

/// usvg 会按引用次数复制内容的功能图标都用不到，直接拒绝；设计工具常见的导出写法照常渲染。
#[test]
fn svg_features_that_copy_content_per_reference_are_refused() {
    let svg = |body: &str| {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:svg="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:e="urn:e" viewBox="0 0 24 24">{body}</svg>"#
        )
    };
    let stops = "<stop/>".repeat(65);
    let cases = [
        (r#"<defs><pattern id="p"/></defs>"#.to_owned(), "<pattern>"),
        (r#"<defs><marker id="m"/></defs>"#.to_owned(), "<marker>"),
        (r#"<filter id="f"/>"#.to_owned(), "<filter>"),
        // 带前缀的也是 SVG 元素。
        (r#"<svg:pattern id="p"/>"#.to_owned(), "<pattern>"),
        (
            r#"<clipPath clipPathUnits="objectBoundingBox"/>"#.to_owned(),
            "clipPathUnits",
        ),
        (
            r#"<clipPath svg:clipPathUnits="objectBoundingBox"/>"#.to_owned(),
            "clipPathUnits",
        ),
        ("<mask/>".to_owned(), "maskUnits"),
        (
            r#"<mask maskUnits="userSpaceOnUse" maskContentUnits="objectBoundingBox"/>"#.to_owned(),
            "maskUnits",
        ),
        (
            r#"<mask maskUnits=" userSpaceOnUse"/>"#.to_owned(),
            "maskUnits",
        ),
        // usvg 不读别的命名空间里的属性，这样写 maskUnits 还是默认的 objectBoundingBox。
        (
            r#"<mask e:maskUnits="userSpaceOnUse"/>"#.to_owned(),
            "maskUnits",
        ),
        // xlink 命名空间里的 usvg 也读。
        (
            r#"<mask maskUnits="userSpaceOnUse" xlink:maskUnits="objectBoundingBox"/>"#.to_owned(),
            "maskUnits",
        ),
        (
            format!("<linearGradient>{stops}</linearGradient>"),
            "最多 64 个",
        ),
    ];
    for (body, expected) in cases {
        let Err(err) = icon(IconFormat::Svg, svg(&body).as_bytes()) else {
            panic!("应被拒绝：{body}")
        };
        assert!(err.contains(expected), "{body}：{err}");
    }

    let exported = svg(r##"<style>.a{fill:url(#g)}</style>
<defs>
  <linearGradient id="g"><stop offset="0" stop-color="#f00"/><stop offset="1" stop-color="#00f"/></linearGradient>
  <clipPath id="c"><rect width="24" height="24"/></clipPath>
  <mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="24" height="24"><rect width="24" height="24" fill="#fff"/></mask>
</defs>
<g clip-path="url(#c)" mask="url(#m)"><rect class="a" width="24" height="24"/></g>"##);
    let pixels = icon_pixels(IconFormat::Svg, exported.as_bytes()).unwrap();
    assert!(!has_transparent_corner(&pixels));
    let [r, _, b, _] = pixels.get_pixel(0, 256).0;
    assert!(r > 200 && b < 50, "渐变从红色开始：{r} {b}");
}

#[test]
fn transparent_corners_are_noticed() {
    let mut pixels = RgbaImage::from_pixel(8, 8, Rgba([1, 2, 3, 255]));
    assert!(!has_transparent_corner(&pixels));
    pixels.put_pixel(7, 7, Rgba([0, 0, 0, 0]));
    assert!(has_transparent_corner(&pixels));
}

#[test]
fn tab_icons_use_alpha_for_shape_and_whiten_rgb() {
    let mut pixels = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 0]));
    for x in 16..48 {
        for y in 16..48 {
            pixels.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
    }
    let icon = tab_icon(IconFormat::Png, &encode_png(pixels).unwrap()).unwrap();
    let rendered = decoded(&icon.large).to_rgba8();
    assert!(rendered.pixels().any(|pixel| pixel[3] == 0));
    assert!(rendered.pixels().any(|pixel| pixel[3] > 0));
    assert!(
        rendered
            .pixels()
            .filter(|pixel| pixel[3] > 0)
            .all(|pixel| pixel.0[..3] == [255, 255, 255])
    );

    let mut colors = RgbaImage::from_pixel(64, 64, Rgba([0, 0, 0, 0]));
    for x in 8..28 {
        for y in 16..48 {
            colors.put_pixel(x, y, Rgba([255, 0, 0, 255]));
        }
    }
    for x in 36..56 {
        for y in 16..48 {
            colors.put_pixel(x, y, Rgba([0, 0, 255, 255]));
        }
    }
    let icon = tab_icon(IconFormat::Png, &encode_png(colors).unwrap()).unwrap();
    let rendered = decoded(&icon.large).to_rgba8();
    assert!(
        rendered
            .pixels()
            .filter(|pixel| pixel[3] > 0)
            .all(|pixel| pixel.0[..3] == [255, 255, 255])
    );

    let opaque = png(256, 256, [0, 0, 0, 255]);
    assert!(
        tab_icon(IconFormat::Png, &opaque)
            .unwrap_err()
            .contains("不能全不透明")
    );
    let transparent = png(256, 256, [0, 0, 0, 0]);
    let error = tab_icon(IconFormat::Png, &transparent).unwrap_err();
    assert!(error.contains("全透明"), "{error}");
}

#[test]
fn screenshots_are_reencoded_as_jpeg_within_1280() {
    let shot = screenshot(&png(2000, 1000, [0, 255, 0, 128])).unwrap();
    assert!(shot.jpeg.starts_with(&[0xff, 0xd8]));
    assert_eq!((shot.width, shot.height), (2000, 1000));
    let image = decoded(&shot.jpeg);
    assert_eq!((image.width(), image.height()), (1280, 640));
    // 半透明的绿色铺在白色上。
    let [r, g, b] = image.to_rgb8().get_pixel(640, 320).0;
    assert!(r > 100 && g > 240 && b > 100, "{r} {g} {b}");

    let small = screenshot(&png(800, 600, [0, 0, 0, 255])).unwrap();
    assert_eq!(decoded(&small.jpeg).width(), 800);
    assert!(screenshot(b"GIF89a").unwrap_err().contains("PNG 或 JPEG"));
    let err = screenshot(&png(4097, 1, [0; 4])).unwrap_err();
    assert!(err.contains("4096"), "{err}");
}
