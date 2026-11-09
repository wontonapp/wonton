//! 对头像的实际字节检查格式、尺寸和时长；不信文件名或上传者给的类型。

use std::io::Cursor;

use wonton_protocol::{
    AvatarMedia, AvatarMediaProblem, MAX_AVATAR_BYTES, MAX_AVATAR_DURATION_MS, MAX_AVATAR_EDGE,
};

mod mp4;
#[cfg(test)]
mod tests;

type Problem = AvatarMediaProblem;
type Metrics = (u32, u32, Option<u32>);

pub fn inspect(data: &[u8]) -> Result<AvatarMedia, AvatarMediaProblem> {
    if data.len() as u64 > MAX_AVATAR_BYTES {
        return Err(Problem::TooLarge);
    }
    let (mimetype, (width, height, duration_ms)) = if data.get(4..8) == Some(b"ftyp") {
        ("video/mp4", mp4::inspect(data)?)
    } else {
        match image::guess_format(data).map_err(|_| Problem::Format)? {
            image::ImageFormat::Png => ("image/png", png(data)?),
            image::ImageFormat::Jpeg => ("image/jpeg", jpeg(data)?),
            image::ImageFormat::Gif => ("image/gif", gif(data)?),
            image::ImageFormat::WebP => ("image/webp", webp(data)?),
            _ => return Err(Problem::Format),
        }
    };
    dimensions(width, height)?;
    Ok(AvatarMedia {
        url: String::new(),
        mimetype: mimetype.into(),
        width,
        height,
        size: data.len() as u64,
        duration_ms,
        poster: None,
    })
}

fn dimensions(width: u32, height: u32) -> Result<(), Problem> {
    if width == 0 || height == 0 {
        return Err(Problem::Unreadable);
    }
    if width != height {
        return Err(Problem::NotSquare);
    }
    if width > MAX_AVATAR_EDGE {
        return Err(Problem::TooBig);
    }
    Ok(())
}

fn duration(ms: u64) -> Result<u32, Problem> {
    if ms > u64::from(MAX_AVATAR_DURATION_MS) {
        return Err(Problem::TooLong);
    }
    Ok(ms as u32)
}

fn png(data: &[u8]) -> Result<Metrics, Problem> {
    let decoder = png::Decoder::new_with_limits(
        Cursor::new(data),
        png::Limits {
            bytes: MAX_AVATAR_BYTES as usize * 4,
        },
    );
    let mut reader = decoder.read_info().map_err(|_| Problem::Unreadable)?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    dimensions(width, height)?;
    // APNG is not a supported animation format; do not silently treat it as static PNG.
    if info.animation_control.is_some() {
        return Err(Problem::Format);
    }
    while reader
        .next_row()
        .map_err(|_| Problem::Unreadable)?
        .is_some()
    {}
    reader.finish().map_err(|_| Problem::Unreadable)?;
    Ok((width, height, None))
}

fn jpeg(data: &[u8]) -> Result<Metrics, Problem> {
    // Unlike image's wrapper this borrows input and retains strict decoding. JPEG needs
    // one output buffer to validate entropy data; PNG/GIF do not allocate whole images.
    let options = zune_core::options::DecoderOptions::default()
        .set_strict_mode(true)
        .set_max_width(u16::MAX as usize)
        .set_max_height(u16::MAX as usize);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(
        zune_core::bytestream::ZCursor::new(data),
        options,
    );
    decoder.decode_headers().map_err(|_| Problem::Unreadable)?;
    let (width, height) = decoder.dimensions().ok_or(Problem::Unreadable)?;
    let (width, height) = (width as u32, height as u32);
    dimensions(width, height)?;
    let mut pixels = vec![0; decoder.output_buffer_size().ok_or(Problem::Unreadable)?];
    decoder
        .decode_into(&mut pixels)
        .map_err(|_| Problem::Unreadable)?;
    // Some JPEG decoders tolerate a missing EOI after complete scan data.
    if !data.windows(2).any(|marker| marker == b"\xff\xd9") {
        return Err(Problem::Unreadable);
    }
    Ok((width, height, None))
}

fn gif(data: &[u8]) -> Result<Metrics, Problem> {
    let mut options = gif::DecodeOptions::new();
    options.check_frame_consistency(true);
    options.check_lzw_end_code(true);
    let mut reader = options.read_info(data).map_err(|_| Problem::Unreadable)?;
    let (width, height) = (u32::from(reader.width()), u32::from(reader.height()));
    dimensions(width, height)?;
    let mut total_ms = 0;
    let mut frames = 0;
    let mut pixels = [0; MAX_AVATAR_EDGE as usize];
    while let Some(frame) = reader.next_frame_info().map_err(|_| Problem::Unreadable)? {
        total_ms += u64::from(frame.delay) * 10;
        duration(total_ms)?;
        frames += 1;
        let (frame_width, frame_height) = (usize::from(frame.width), frame.height);
        if frame_width == 0 || frame_height == 0 || frame_width > pixels.len() {
            return Err(Problem::Unreadable);
        }
        for _ in 0..frame_height {
            if !reader
                .fill_buffer(&mut pixels[..frame_width])
                .map_err(|_| Problem::Unreadable)?
            {
                return Err(Problem::Unreadable);
            }
        }
        if reader
            .fill_buffer(&mut pixels[..1])
            .map_err(|_| Problem::Unreadable)?
        {
            return Err(Problem::Unreadable);
        }
    }
    if frames == 0 || data.last() != Some(&0x3b) {
        return Err(Problem::Unreadable);
    }
    Ok((width, height, Some(duration(total_ms)?)))
}

fn webp(data: &[u8]) -> Result<Metrics, Problem> {
    // The decoder seeks over chunks and tolerates EOF at a chunk boundary. Check
    // RIFF's complete declared extent before asking it for metrics or frame data.
    let declared = data.get(4..8).ok_or(Problem::Unreadable)?;
    let declared = u32::from_le_bytes(declared.try_into().map_err(|_| Problem::Unreadable)?);
    if u64::from(declared) + 8 != data.len() as u64 {
        return Err(Problem::Unreadable);
    }
    let frames = webp_chunks(data.get(12..).ok_or(Problem::Unreadable)?, false)?;
    let mut decoder =
        image_webp::WebPDecoder::new(Cursor::new(data)).map_err(|_| Problem::Unreadable)?;
    let (width, height) = decoder.dimensions();
    dimensions(width, height)?;
    if decoder.is_animated() != (frames > 0) || decoder.num_frames() != frames {
        return Err(Problem::Format);
    }
    let duration_ms = if decoder.is_animated() {
        if decoder.num_frames() == 0 {
            return Err(Problem::Unreadable);
        }
        Some(duration(decoder.loop_duration())?)
    } else {
        None
    };
    // Reuse one buffer: image::AnimationDecoder allocates and composites a new
    // full RGBA frame for every delay, even when only metadata is wanted.
    let channels = if decoder.has_alpha() { 4 } else { 3 };
    let mut pixels = vec![0; width as usize * height as usize * channels];
    if decoder.is_animated() {
        for _ in 0..decoder.num_frames() {
            decoder
                .read_frame(&mut pixels)
                .map_err(|_| Problem::Unreadable)?;
        }
    } else {
        decoder
            .read_image(&mut pixels)
            .map_err(|_| Problem::Unreadable)?;
    }
    Ok((width, height, duration_ms))
}

fn webp_chunks(mut chunks: &[u8], inside_frame: bool) -> Result<u32, Problem> {
    let mut frames = 0;
    while !chunks.is_empty() {
        let header = chunks.get(..8).ok_or(Problem::Unreadable)?;
        let size =
            u32::from_le_bytes(header[4..8].try_into().map_err(|_| Problem::Unreadable)?) as usize;
        let padded = size.checked_add(size & 1).ok_or(Problem::Unreadable)?;
        let end = 8usize.checked_add(padded).ok_or(Problem::Unreadable)?;
        let body = chunks.get(8..8 + size).ok_or(Problem::Unreadable)?;
        if &header[..4] == b"ANMF" {
            if inside_frame {
                return Err(Problem::Unreadable);
            }
            webp_chunks(body.get(16..).ok_or(Problem::Unreadable)?, true)?;
            frames += 1;
        }
        chunks = chunks.get(end..).ok_or(Problem::Unreadable)?;
    }
    Ok(frames)
}
