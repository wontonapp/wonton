use std::io::Cursor;

use mp4parse::{CodecType, MediaContext, SampleEntry, Track, TrackType, VideoCodecSpecific};

use super::{Metrics, Problem, dimensions, duration};

mod fragments;

/// 扫出来的顶层 box：一个 `moov` 和它的原始字节，`mdat` 里可以放样本的区间，还有 `moof` 片段。
struct Scan<'a> {
    movie: &'a [u8],
    movie_box: &'a [u8],
    media_ranges: Vec<std::ops::Range<usize>>,
    fragments: Vec<(usize, &'a [u8])>,
}

/// 按谁的要求检查。头像要正方、不超过 10 分钟、只认 H.264（ADR-0044）；`view_file` 只报时长和分辨率，这些都不算数。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Limits {
    Avatar,
    View,
}

/// 视频的时长和分辨率，给 `view_file` 用。
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
}

pub(super) fn inspect(data: &[u8]) -> Result<Metrics, Problem> {
    let scan = scan(data)?;
    let duration_ms = movie_duration(scan.movie)?;
    check_video_formats(scan.movie)?;
    let context = parse_movie(scan.movie_box)?;
    let (width, height) = video_sizes(&context, &scan, Limits::Avatar)?;
    Ok((width, height, Some(duration(duration_ms)?)))
}

/// 用户视频的探针：头像那套限制放宽，错误照旧按 [`Problem`] 报，由调用方映射。
pub(crate) fn probe(data: &[u8]) -> Result<VideoInfo, Problem> {
    let scan = scan(data)?;
    let duration_ms = movie_duration(scan.movie)?;
    let context = parse_movie(scan.movie_box)?;
    let (width, height) = video_sizes(&context, &scan, Limits::View)?;
    Ok(VideoInfo {
        width,
        height,
        duration_ms,
    })
}

fn scan(data: &[u8]) -> Result<Scan<'_>, Problem> {
    let mut remaining = data;
    let mut movie = None;
    let mut media_ranges = Vec::new();
    let mut fragments = Vec::new();
    while !remaining.is_empty() {
        let offset = data.len() - remaining.len();
        let (kind, body, header) = next_box(&mut remaining)?;
        match kind {
            b"moov" if movie.is_none() => {
                movie = Some((body, &data[offset..offset + header + body.len()]))
            }
            b"moov" => return Err(Problem::Unreadable),
            b"mdat" => media_ranges.push(offset + header..offset + header + body.len()),
            b"moof" => fragments.push((offset, body)),
            _ => {}
        }
    }
    let (movie, movie_box) = movie.ok_or(Problem::Unreadable)?;
    if media_ranges.iter().all(std::ops::Range::is_empty) {
        return Err(Problem::Unreadable);
    }
    Ok(Scan {
        movie,
        movie_box,
        media_ranges,
        fragments,
    })
}

// mp4parse only needs moov for metrics and rejects a legal EOF-sized mdat.
// Parse the borrowed movie box; top-level extents are already checked above.
fn parse_movie(movie_box: &[u8]) -> Result<MediaContext, Problem> {
    mp4parse::read_mp4(&mut Cursor::new(movie_box)).map_err(|_| Problem::Unreadable)
}

/// 所有视频轨的样本尺寸（有两条不一样的就算读不懂），顺带把每条轨的样本字节核对进 `mdat`。
fn video_sizes(
    context: &MediaContext,
    scan: &Scan<'_>,
    limits: Limits,
) -> Result<(u32, u32), Problem> {
    let mut video = None;
    for track in context
        .tracks
        .iter()
        .filter(|t| t.track_type == TrackType::Video)
    {
        let descriptions = &track.stsd.as_ref().ok_or(Problem::Unreadable)?.descriptions;
        if descriptions.is_empty() {
            return Err(Problem::Unreadable);
        }
        for entry in descriptions.iter() {
            let SampleEntry::Video(entry) = entry else {
                return Err(Problem::Unreadable);
            };
            if limits == Limits::Avatar {
                if entry.codec_type != CodecType::H264 {
                    return Err(Problem::Format);
                }
                let VideoCodecSpecific::AVCConfig(config) = &entry.codec_specific else {
                    return Err(Problem::Format);
                };
                if config.len() < 7 || config[0] != 1 {
                    return Err(Problem::Unreadable);
                }
            }
            let size = (u32::from(entry.width), u32::from(entry.height));
            match limits {
                Limits::Avatar => dimensions(size.0, size.1)?,
                Limits::View if size.0 == 0 || size.1 == 0 => return Err(Problem::Unreadable),
                Limits::View => {}
            }
            if video.is_some_and(|previous| previous != size) {
                return Err(Problem::Unreadable);
            }
            video = Some(size);
        }
        if scan.fragments.is_empty() || track.stts.as_ref().is_some_and(|t| !t.samples.is_empty()) {
            validate_samples(track, &scan.media_ranges)?;
        } else {
            fragments::validate(
                &scan.fragments,
                scan.movie,
                track.track_id.ok_or(Problem::Unreadable)?,
                &scan.media_ranges,
            )?;
        }
    }
    video.ok_or(Problem::Unreadable)
}

// mp4parse supports avc1/avc3 and validates track boxes, but intentionally discards
// mvhd.duration. Read only that metric here rather than guessing from mdhd or tkhd.
fn movie_duration(mut movie: &[u8]) -> Result<u64, Problem> {
    let mut result = None;
    while !movie.is_empty() {
        let (kind, body, _) = next_box(&mut movie)?;
        if kind != b"mvhd" {
            continue;
        }
        if result.is_some() {
            return Err(Problem::Unreadable);
        }
        let (scale, ticks) = match body.first() {
            Some(0) => (u64::from(be32(body, 12)?), u64::from(be32(body, 16)?)),
            Some(1) => (u64::from(be32(body, 20)?), be64(body, 24)?),
            _ => return Err(Problem::Unreadable),
        };
        if scale == 0
            || ticks == 0
            || ticks == u64::MAX
            || (body.first() == Some(&0) && ticks == u64::from(u32::MAX))
        {
            return Err(Problem::Unreadable);
        }
        // Round up: 6000.01ms must not become a passing 6000ms.
        let ms = ticks
            .checked_mul(1000)
            .and_then(|n| n.checked_add(scale - 1))
            .ok_or(Problem::Unreadable)?
            / scale;
        result = Some(ms);
    }
    result.ok_or(Problem::Unreadable)
}

fn next_box<'a>(input: &mut &'a [u8]) -> Result<(&'a [u8], &'a [u8], usize), Problem> {
    let size = be32(input, 0)?;
    let kind = input.get(4..8).ok_or(Problem::Unreadable)?;
    let (size, header) = match size {
        0 => (input.len(), 8),
        1 => (
            usize::try_from(be64(input, 8)?).map_err(|_| Problem::Unreadable)?,
            16,
        ),
        size => (size as usize, 8),
    };
    if size < header || size > input.len() {
        return Err(Problem::Unreadable);
    }
    let body = &input[header..size];
    *input = &input[size..];
    Ok((kind, body, header))
}

fn be32(data: &[u8], offset: usize) -> Result<u32, Problem> {
    let bytes = data.get(offset..offset + 4).ok_or(Problem::Unreadable)?;
    Ok(u32::from_be_bytes(
        bytes.try_into().map_err(|_| Problem::Unreadable)?,
    ))
}

fn be64(data: &[u8], offset: usize) -> Result<u64, Problem> {
    let bytes = data.get(offset..offset + 8).ok_or(Problem::Unreadable)?;
    Ok(u64::from_be_bytes(
        bytes.try_into().map_err(|_| Problem::Unreadable)?,
    ))
}

// mp4parse maps both unknown codecs and missing codec configuration to Unknown.
// Check only sample-entry tags here to distinguish Format from Unreadable.
fn check_video_formats(mut movie: &[u8]) -> Result<(), Problem> {
    while !movie.is_empty() {
        let (kind, track, _) = next_box(&mut movie)?;
        if kind != b"trak" {
            continue;
        }
        let media = child(track, b"mdia")?;
        let handler = child(media, b"hdlr")?;
        if handler.get(8..12) != Some(b"vide") {
            continue;
        }
        let table = child(child(media, b"minf")?, b"stbl")?;
        let descriptions = child(table, b"stsd")?;
        let count = be32(descriptions, 4)?;
        let mut entries = descriptions.get(8..).ok_or(Problem::Unreadable)?;
        for _ in 0..count {
            let (codec, body, _) = next_box(&mut entries)?;
            if codec != b"avc1" && codec != b"avc3" {
                return Err(Problem::Format);
            }
            if body.len() < 78 || body.get(6..8) != Some(b"\0\x01") {
                return Err(Problem::Unreadable);
            }
        }
    }
    Ok(())
}

fn child<'a>(mut parent: &'a [u8], name: &[u8; 4]) -> Result<&'a [u8], Problem> {
    while !parent.is_empty() {
        let (kind, body, _) = next_box(&mut parent)?;
        if kind == name {
            return Ok(body);
        }
    }
    Err(Problem::Unreadable)
}

// A complete moov alone is not a readable video. Validate each video sample's
// byte range against mdat, including fast-start files truncated after moov.
fn validate_samples(track: &Track, media: &[std::ops::Range<usize>]) -> Result<(), Problem> {
    let chunks = &track.stco.as_ref().ok_or(Problem::Unreadable)?.offsets;
    let mapping = &track.stsc.as_ref().ok_or(Problem::Unreadable)?.samples;
    let sizes = track.stsz.as_ref().ok_or(Problem::Unreadable)?;
    let timing = &track.stts.as_ref().ok_or(Problem::Unreadable)?.samples;
    let count = timing.iter().try_fold(0u64, |n, entry| {
        n.checked_add(u64::from(entry.sample_count))
            .ok_or(Problem::Unreadable)
    })?;
    if count == 0 || chunks.is_empty() || mapping.first().is_none_or(|m| m.first_chunk != 1) {
        return Err(Problem::Unreadable);
    }
    if sizes.sample_size == 0 && sizes.sample_sizes.len() as u64 != count {
        return Err(Problem::Unreadable);
    }
    if mapping
        .windows(2)
        .any(|m| m[0].first_chunk >= m[1].first_chunk)
    {
        return Err(Problem::Unreadable);
    }
    let descriptions = track
        .stsd
        .as_ref()
        .ok_or(Problem::Unreadable)?
        .descriptions
        .len();
    let mut map_index = 0;
    let mut sample_index = 0u64;
    for (index, offset) in chunks.iter().enumerate() {
        while mapping
            .get(map_index + 1)
            .is_some_and(|m| m.first_chunk as usize <= index + 1)
        {
            map_index += 1;
        }
        let map = &mapping[map_index];
        if map.samples_per_chunk == 0
            || map.sample_description_index == 0
            || map.sample_description_index as usize > descriptions
        {
            return Err(Problem::Unreadable);
        }
        let end_index = sample_index + u64::from(map.samples_per_chunk);
        if end_index > count {
            return Err(Problem::Unreadable);
        }
        let mut end = *offset;
        for sample in sample_index..end_index {
            let size = if sizes.sample_size == 0 {
                sizes.sample_sizes[sample as usize]
            } else {
                sizes.sample_size
            };
            if size == 0 {
                return Err(Problem::Unreadable);
            }
            end = end
                .checked_add(u64::from(size))
                .ok_or(Problem::Unreadable)?;
        }
        if !media
            .iter()
            .any(|r| *offset >= r.start as u64 && end <= r.end as u64)
        {
            return Err(Problem::Unreadable);
        }
        sample_index = end_index;
    }
    if sample_index != count {
        return Err(Problem::Unreadable);
    }
    Ok(())
}
