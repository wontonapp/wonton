use super::*;

const PNG: &[u8] = include_bytes!("../../tests/fixtures/avatars/square.png");
const JPEG: &[u8] = include_bytes!("../../tests/fixtures/avatars/square.jpg");
const WEBP: &[u8] = include_bytes!("../../tests/fixtures/avatars/square.webp");
const GIF: &[u8] = include_bytes!("../../tests/fixtures/avatars/animated.gif");
const ANIMATED_WEBP: &[u8] = include_bytes!("../../tests/fixtures/avatars/animated.webp");
const MP4: &[u8] = include_bytes!("../../tests/fixtures/avatars/h264.mp4");

// ImageMagick: magick -size 16x16 xc:red square.{png,jpg,webp}
// magick -delay 10 -size 16x16 xc:red -delay 20 -size 16x16 xc:blue -loop 0 animated.{gif,webp}
// ffmpeg -f lavfi -i color=c=red:s=16x16:r=2:d=0.5 -c:v libx264 -pix_fmt yuv420p
//   -threads 1 -movflags +faststart h264.mp4 (avc3: -tag:v avc3; rejected codec: -c:v mpeg4)
// Limit/rectangle fixtures use the same solid-color generation at their named dimensions.
#[test]
fn reads_real_supported_media_and_leaves_matrix_fields_unset() {
    let avc3 = include_bytes!("../../tests/fixtures/avatars/h264-avc3.mp4");
    for (bytes, mime, duration) in [
        (PNG, "image/png", None),
        (JPEG, "image/jpeg", None),
        (WEBP, "image/webp", None),
        (GIF, "image/gif", Some(300)),
        (ANIMATED_WEBP, "image/webp", Some(300)),
        (MP4, "video/mp4", Some(500)),
        (avc3.as_slice(), "video/mp4", Some(500)),
    ] {
        let media = inspect(bytes).unwrap();
        assert_eq!((media.width, media.height), (16, 16));
        assert_eq!(media.mimetype, mime);
        assert_eq!(media.duration_ms, duration);
        assert_eq!(media.size, bytes.len() as u64);
        assert!(media.url.is_empty());
        assert_eq!(media.poster, None);
    }
}

#[test]
fn rejects_unsupported_images_and_video_codecs() {
    let bmp = include_bytes!("../../tests/fixtures/avatars/unsupported.bmp");
    let mpeg4 = include_bytes!("../../tests/fixtures/avatars/mpeg4.mp4");
    assert_eq!(inspect(bmp), Err(Problem::Format));
    assert_eq!(inspect(mpeg4), Err(Problem::Format));
    let apng = include_bytes!("../../tests/fixtures/avatars/unsupported-apng.png");
    assert_eq!(inspect(apng), Err(Problem::Format));
    assert_eq!(inspect(b"not a media file"), Err(Problem::Format));
}

#[test]
fn enforces_square_and_exact_edge_limit() {
    let limit = include_bytes!("../../tests/fixtures/avatars/limit.png");
    let oversized = include_bytes!("../../tests/fixtures/avatars/oversized.png");
    let rectangle = include_bytes!("../../tests/fixtures/avatars/rectangle.png");
    assert_eq!(inspect(limit).unwrap().width, MAX_AVATAR_EDGE);
    assert_eq!(inspect(oversized), Err(Problem::TooBig));
    assert_eq!(inspect(rectangle), Err(Problem::NotSquare));
}

#[test]
fn enforces_exact_byte_limit_without_decoding_oversized_input() {
    let mut bytes = JPEG.to_vec();
    bytes.resize(MAX_AVATAR_BYTES as usize, 0);
    assert_eq!(inspect(&bytes).unwrap().size, MAX_AVATAR_BYTES);
    bytes.push(0);
    assert_eq!(inspect(&bytes), Err(Problem::TooLarge));
}

#[test]
fn sums_all_gif_frame_delays_and_accepts_exact_duration_limit() {
    let mut bytes = GIF.to_vec();
    let controls: Vec<_> = bytes
        .windows(3)
        .enumerate()
        .filter_map(|(i, tag)| (tag == b"\x21\xf9\x04").then_some(i))
        .collect();
    assert_eq!(controls.len(), 2);
    for i in &controls {
        bytes[i + 4..i + 6].copy_from_slice(&300u16.to_le_bytes());
    }
    assert_eq!(
        inspect(&bytes).unwrap().duration_ms,
        Some(MAX_AVATAR_DURATION_MS)
    );
    let i = controls[1];
    bytes[i + 4..i + 6].copy_from_slice(&301u16.to_le_bytes());
    assert_eq!(inspect(&bytes), Err(Problem::TooLong));
}

#[test]
fn sums_all_webp_anmf_delays_and_accepts_exact_duration_limit() {
    let mut bytes = ANIMATED_WEBP.to_vec();
    let frames: Vec<_> = bytes
        .windows(4)
        .enumerate()
        .filter_map(|(i, tag)| (tag == b"ANMF").then_some(i))
        .collect();
    assert_eq!(frames.len(), 2);
    for i in &frames {
        bytes[i + 20..i + 23].copy_from_slice(&3000u32.to_le_bytes()[..3]);
    }
    assert_eq!(
        inspect(&bytes).unwrap().duration_ms,
        Some(MAX_AVATAR_DURATION_MS)
    );
    let i = frames[1];
    bytes[i + 20..i + 23].copy_from_slice(&3001u32.to_le_bytes()[..3]);
    assert_eq!(inspect(&bytes), Err(Problem::TooLong));
}

#[test]
fn mp4_uses_mvhd_not_track_duration_and_rejects_fraction_over_limit() {
    let mut bytes = MP4.to_vec();
    let mvhd = tag(&bytes, b"mvhd");
    bytes[mvhd + 16..mvhd + 20].copy_from_slice(&100_000u32.to_be_bytes());
    bytes[mvhd + 20..mvhd + 24].copy_from_slice(&600_000u32.to_be_bytes());
    assert_eq!(
        inspect(&bytes).unwrap().duration_ms,
        Some(MAX_AVATAR_DURATION_MS)
    );
    bytes[mvhd + 20..mvhd + 24].copy_from_slice(&600_001u32.to_be_bytes());
    assert_eq!(inspect(&bytes), Err(Problem::TooLong));
}

#[test]
fn mp4_uses_video_sample_dimensions_not_track_display_size() {
    let mut bytes = MP4.to_vec();
    let avc = bytes
        .windows(8)
        .position(|part| part == b"avc1\0\0\0\0")
        .unwrap();
    bytes[avc + 28..avc + 30].copy_from_slice(&8u16.to_be_bytes());
    assert_eq!(inspect(&bytes), Err(Problem::NotSquare));
    bytes[avc + 28..avc + 32].copy_from_slice(&[0, 0, 0, 0]);
    assert_eq!(inspect(&bytes), Err(Problem::Unreadable));
}

#[test]
fn truncated_supported_files_are_unreadable_not_unsupported() {
    for bytes in [PNG, JPEG, WEBP, GIF, ANIMATED_WEBP, MP4] {
        assert_eq!(inspect(&bytes[..bytes.len() / 2]), Err(Problem::Unreadable));
        assert_eq!(inspect(&bytes[..bytes.len() - 1]), Err(Problem::Unreadable));
    }
}

#[test]
fn damaged_payloads_are_rejected_after_recognizing_the_header() {
    let mut png = PNG.to_vec();
    let idat = tag(&png, b"IDAT");
    png[idat + 4] ^= 0xff;
    assert_eq!(inspect(&png), Err(Problem::Unreadable));
    let mut webp = WEBP.to_vec();
    webp[20..].fill(0);
    assert_eq!(inspect(&webp), Err(Problem::Unreadable));
    let mut gif = GIF.to_vec();
    gif[6..10].fill(0);
    assert_eq!(inspect(&gif), Err(Problem::Unreadable));
    let mut mp4 = MP4.to_vec();
    let mvhd = tag(&mp4, b"mvhd");
    mp4[mvhd + 16..mvhd + 20].fill(0);
    assert_eq!(inspect(&mp4), Err(Problem::Unreadable));
}

#[test]
fn mp4_missing_metrics_and_media_bytes_are_unreadable() {
    for kind in [b"mvhd", b"stsd", b"mdat", b"avcC"] {
        let mut bytes = MP4.to_vec();
        let offset = tag(&bytes, kind);
        bytes[offset..offset + 4].copy_from_slice(b"free");
        assert_eq!(inspect(&bytes), Err(Problem::Unreadable));
    }
    let mut bytes = MP4.to_vec();
    let offset = tag(&bytes, b"stco");
    bytes[offset + 12..offset + 16].copy_from_slice(&1u32.to_be_bytes());
    assert_eq!(inspect(&bytes), Err(Problem::Unreadable));
}

#[test]
fn reads_64_bit_mvhd_duration_without_using_track_timescale() {
    let bytes = include_bytes!("../../tests/fixtures/avatars/h264-mvhd-v1.mp4");
    assert_eq!(inspect(bytes).unwrap().duration_ms, Some(5000));
    let mut bytes = bytes.to_vec();
    let mvhd = tag(&bytes, b"mvhd");
    bytes[mvhd + 28..mvhd + 36].copy_from_slice(&6_000_000_001u64.to_be_bytes());
    assert_eq!(inspect(&bytes), Err(Problem::TooLong));
}

#[test]
fn rejects_zero_or_unknown_mp4_duration_and_truncated_box_headers() {
    for ticks in [0u32, u32::MAX] {
        let mut bytes = MP4.to_vec();
        let mvhd = tag(&bytes, b"mvhd");
        bytes[mvhd + 20..mvhd + 24].copy_from_slice(&ticks.to_be_bytes());
        assert_eq!(inspect(&bytes), Err(Problem::Unreadable));
    }
    let mut bytes = MP4.to_vec();
    bytes.extend_from_slice(b"\0\0\0");
    assert_eq!(inspect(&bytes), Err(Problem::Unreadable));
}

#[test]
fn webp_truncated_frame_is_unreadable_even_with_adjusted_riff_size() {
    let mut bytes = ANIMATED_WEBP[..ANIMATED_WEBP.len() - 2].to_vec();
    let size = bytes.len() as u32 - 8;
    bytes[4..8].copy_from_slice(&size.to_le_bytes());
    assert_eq!(inspect(&bytes), Err(Problem::Unreadable));
}

fn tag(bytes: &[u8], kind: &[u8; 4]) -> usize {
    bytes.windows(4).position(|part| part == kind).unwrap()
}

#[test]
fn accepts_eof_sized_media_and_fragmented_h264_with_known_movie_duration() {
    let mut eof = MP4.to_vec();
    let mdat = tag(&eof, b"mdat");
    eof[mdat - 4..mdat].fill(0);
    assert_eq!(inspect(&eof).unwrap().duration_ms, Some(500));
    // ffmpeg +empty_moov+default_base_moof+frag_keyframe; mvhd populated to 500ms.
    let fragmented = include_bytes!("../../tests/fixtures/avatars/h264-fragmented.mp4");
    assert_eq!(inspect(fragmented).unwrap().duration_ms, Some(500));
    let mut broken = fragmented.to_vec();
    let trun = tag(&broken, b"trun");
    broken[trun + 12..trun + 16].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(inspect(&broken), Err(Problem::Unreadable));
}

#[test]
fn view_probe_takes_long_rectangle_videos() {
    let mut bytes = MP4.to_vec();
    let mvhd = tag(&bytes, b"mvhd");
    bytes[mvhd + 16..mvhd + 20].copy_from_slice(&1000u32.to_be_bytes());
    bytes[mvhd + 20..mvhd + 24].copy_from_slice(&120_000u32.to_be_bytes());
    let avc = bytes
        .windows(8)
        .position(|part| part == b"avc1\0\0\0\0")
        .unwrap();
    bytes[avc + 28..avc + 30].copy_from_slice(&32u16.to_be_bytes());
    bytes[avc + 30..avc + 32].copy_from_slice(&16u16.to_be_bytes());
    let info = probe_video(&bytes).unwrap();
    assert_eq!((info.width, info.height), (32, 16));
    assert_eq!(info.duration_ms, 120_000);
    // 头像那套照旧：不是正方，也只能是 6 秒以内。
    assert_eq!(inspect(&bytes), Err(Problem::NotSquare));
}

#[test]
fn view_probe_accepts_other_codecs_but_the_avatar_check_keeps_h264() {
    let mpeg4 = include_bytes!("../../tests/fixtures/avatars/mpeg4.mp4");
    assert_eq!(probe_video(mpeg4).unwrap().width, 16);
    assert_eq!(inspect(mpeg4), Err(Problem::Format));
    assert_eq!(probe_video(PNG), Err(Problem::Format));
}
