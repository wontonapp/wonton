use std::ops::Range;

use super::{Problem, be32, be64, child, next_box};

struct Defaults {
    size: u32,
    base: u64,
}

pub(super) fn validate(
    fragments: &[(usize, &[u8])],
    movie: &[u8],
    track: u32,
    media: &[Range<usize>],
) -> Result<(), Problem> {
    let mut found = false;
    for (offset, fragment) in fragments {
        let mut boxes = *fragment;
        let mut implicit_base = *offset as u64;
        while !boxes.is_empty() {
            let (kind, traf, _) = next_box(&mut boxes)?;
            if kind != b"traf" {
                continue;
            }
            let header = child(traf, b"tfhd")?;
            let id = be32(header, 4)?;
            let defaults = defaults(header, movie, *offset as u64, implicit_base)?;
            let (end, samples) = runs(traf, defaults, media)?;
            implicit_base = end;
            found |= id == track && samples;
        }
    }
    if found {
        Ok(())
    } else {
        Err(Problem::Unreadable)
    }
}

fn defaults(header: &[u8], movie: &[u8], moof: u64, implicit: u64) -> Result<Defaults, Problem> {
    let flags = flags(header)?;
    let track = be32(header, 4)?;
    let mut cursor = 8;
    let base = if flags & 1 != 0 {
        let base = be64(header, cursor)?;
        cursor += 8;
        base
    } else if flags & 0x020000 != 0 {
        moof
    } else {
        implicit
    };
    if flags & 2 != 0 {
        cursor += 4;
    }
    if flags & 8 != 0 {
        cursor += 4;
    }
    let size = if flags & 0x10 != 0 {
        let size = be32(header, cursor)?;
        cursor += 4;
        size
    } else {
        default_size(movie, track)?
    };
    if flags & 0x20 != 0 {
        cursor += 4;
    }
    if cursor != header.len() {
        return Err(Problem::Unreadable);
    }
    Ok(Defaults { size, base })
}

fn default_size(movie: &[u8], track: u32) -> Result<u32, Problem> {
    let mut boxes = child(movie, b"mvex")?;
    while !boxes.is_empty() {
        let (kind, body, _) = next_box(&mut boxes)?;
        if kind == b"trex" && be32(body, 4)? == track {
            return be32(body, 16);
        }
    }
    Err(Problem::Unreadable)
}

fn runs(
    mut traf: &[u8],
    defaults: Defaults,
    media: &[Range<usize>],
) -> Result<(u64, bool), Problem> {
    let mut end = defaults.base;
    let mut found = false;
    while !traf.is_empty() {
        let (kind, body, _) = next_box(&mut traf)?;
        if kind != b"trun" {
            continue;
        }
        let flags = flags(body)?;
        let count = be32(body, 4)?;
        let mut cursor = 8;
        let start = if flags & 1 != 0 {
            let offset = i32::from_be_bytes(be32(body, cursor)?.to_be_bytes());
            cursor += 4;
            defaults
                .base
                .checked_add_signed(i64::from(offset))
                .ok_or(Problem::Unreadable)?
        } else {
            end
        };
        if flags & 4 != 0 {
            cursor += 4;
        }
        let bytes = sample_bytes(body, cursor, flags, count, defaults.size)?;
        end = start.checked_add(bytes).ok_or(Problem::Unreadable)?;
        if count != 0
            && !media
                .iter()
                .any(|range| start >= range.start as u64 && end <= range.end as u64)
        {
            return Err(Problem::Unreadable);
        }
        found |= count != 0;
    }
    Ok((end, found))
}

fn sample_bytes(
    body: &[u8],
    cursor: usize,
    flags: u32,
    count: u32,
    default: u32,
) -> Result<u64, Problem> {
    let stride = ((flags >> 8) & 0xf).count_ones() as usize * 4;
    let entries = body.get(cursor..).ok_or(Problem::Unreadable)?;
    if entries.len() as u64 != u64::from(count) * stride as u64 {
        return Err(Problem::Unreadable);
    }
    if flags & 0x200 == 0 {
        if count > 0 && default == 0 {
            return Err(Problem::Unreadable);
        }
        return Ok(u64::from(count) * u64::from(default));
    }
    let size_offset = usize::from(flags & 0x100 != 0) * 4;
    entries
        .chunks_exact(stride)
        .try_fold(0u64, |total, sample| {
            let size = be32(sample, size_offset)?;
            if size == 0 {
                return Err(Problem::Unreadable);
            }
            total
                .checked_add(u64::from(size))
                .ok_or(Problem::Unreadable)
        })
}

fn flags(body: &[u8]) -> Result<u32, Problem> {
    let value = be32(body, 0)?;
    if value >> 24 > 1 {
        return Err(Problem::Unreadable);
    }
    Ok(value & 0xffffff)
}
