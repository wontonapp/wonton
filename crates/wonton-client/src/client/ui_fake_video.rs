//! 假数据入口里的影视页：片库、首页几排、海报墙和详情都写死，片名是编的。图片是本机一个小服务现画的渐变色
//! PNG，走和真核心一样的 `http://127.0.0.1` 地址，壳的图片加载和缓存照常跑。没有片源，起播回“连不上”。

use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::OnceLock;

use http_body_util::Full;
use hyper::body::{Bytes, Incoming};
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use wonton_protocol::media::{
    MediaFailure, MediaFailureCode, MediaItemRef, MediaKind, MediaTrack, TrackKind,
};

use crate::media_link::{
    OfflineState, VideoDetail, VideoEntry, VideoEntryKind, VideoHome, VideoLibrary,
    VideoLibraryKind, VideoMediaInfo, VideoPerson, VideoRole, VideoShelf, VideoShelfKind,
    VideoWall, VideoWallQuery,
};

const MOVIES: [(&str, &str, u16); 12] = [
    ("m1", "The Lighthouse Keeper", 2024),
    ("m2", "Letters from the North", 2023),
    ("m3", "Galaxy Express", 2025),
    ("m4", "The Seventh Summer", 2022),
    ("m5", "Old Hill Town", 2021),
    ("m6", "Harbor in the Fog", 2024),
    ("m7", "Night Drive", 2020),
    ("m8", "Paper Planes", 2019),
    ("m9", "Whale Station", 2025),
    ("m10", "Island Diary", 2023),
    ("m11", "The Platform", 2018),
    ("m12", "Late Spring", 2022),
];
const SERIES: [(&str, &str, u16); 2] = [
    ("s1", "Small Town Doctor", 2023),
    ("s2", "Deep Sea Base", 2025),
];
const EPISODES: [&str; 6] = [
    "Arrival",
    "Rainy Season",
    "Night Shift",
    "Old Records",
    "Blackout",
    "Farewell",
];

/// 画图的小服务；第一次要图时起，进程里只起一个。
static ART: OnceLock<String> = OnceLock::new();

fn art_prefix() -> &'static str {
    ART.get_or_init(|| {
        let listener = std::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .expect("ui-fake art server binds a loopback port");
        let addr = listener
            .local_addr()
            .expect("bound listener has an address");
        listener
            .set_nonblocking(true)
            .expect("ui-fake art listener goes non-blocking");
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("ui-fake art runtime builds");
            runtime.block_on(serve(listener));
        });
        format!("http://{addr}/art")
    })
}

async fn serve(listener: std::net::TcpListener) {
    let listener =
        tokio::net::TcpListener::from_std(listener).expect("ui-fake art listener joins tokio");
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(async move {
            let service = service_fn(|request: Request<Incoming>| async move {
                Ok::<_, Infallible>(picture(request.uri().path()))
            });
            let _ = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

/// `/art/<种子>/<宽>x<高>.png`：按种子挑两种颜色画竖向渐变。
fn picture(path: &str) -> Response<Full<Bytes>> {
    let parsed = path.strip_prefix("/art/").and_then(|rest| {
        let (seed, size) = rest.split_once('/')?;
        let (w, h) = size.strip_suffix(".png")?.split_once('x')?;
        Some((
            seed.to_owned(),
            w.parse::<u32>().ok()?,
            h.parse::<u32>().ok()?,
        ))
    });
    let Some((seed, w, h)) = parsed.filter(|(_, w, h)| *w <= 1600 && *h <= 1600) else {
        let mut response = Response::new(Full::new(Bytes::new()));
        *response.status_mut() = StatusCode::NOT_FOUND;
        return response;
    };
    let mut response = Response::new(Full::new(Bytes::from(gradient_png(&seed, w, h))));
    response
        .headers_mut()
        .insert("content-type", "image/png".parse().expect("static header"));
    response
}

fn palette(seed: &str) -> ([u8; 3], [u8; 3]) {
    const COLORS: [([u8; 3], [u8; 3]); 6] = [
        ([0x1d, 0x3b, 0x53], [0xe0, 0x8e, 0x45]),
        ([0x2b, 0x1d, 0x3f], [0xc9, 0x4b, 0x7c]),
        ([0x0f, 0x3d, 0x3e], [0x7f, 0xc8, 0xa9]),
        ([0x3a, 0x2c, 0x1e], [0xd9, 0xb5, 0x6c]),
        ([0x14, 0x23, 0x3c], [0x6c, 0x9b, 0xd9]),
        ([0x40, 0x16, 0x16], [0xe5, 0x6b, 0x4e]),
    ];
    let hash = seed
        .bytes()
        .fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(u32::from(b)));
    COLORS[(hash as usize) % COLORS.len()]
}

/// 不压缩的 PNG（zlib 存储块），够假数据用。
fn gradient_png(seed: &str, w: u32, h: u32) -> Vec<u8> {
    let (top, bottom) = palette(seed);
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for y in 0..h {
        raw.push(0);
        let t = y * 255 / h.max(1);
        let mix = |a: u8, b: u8| {
            let (a, b) = (u32::from(a), u32::from(b));
            u8::try_from((a * (255 - t) + b * t) / 255).unwrap_or(u8::MAX)
        };
        let row = [
            mix(top[0], bottom[0]),
            mix(top[1], bottom[1]),
            mix(top[2], bottom[2]),
        ];
        for _ in 0..w {
            raw.extend_from_slice(&row);
        }
    }
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&w.to_be_bytes());
    header.extend_from_slice(&h.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = data.chunks(65_535).collect();
    for (i, block) in blocks.iter().enumerate() {
        out.push(u8::from(i + 1 == blocks.len()));
        let len = u16::try_from(block.len()).expect("blocks are at most 65535 bytes");
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for byte in data {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = png[start..].iter().fold(!0u32, |mut crc, byte| {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
        crc
    });
    png.extend_from_slice(&(!crc).to_be_bytes());
}

fn art(seed: &str, w: u32, h: u32) -> Option<String> {
    Some(format!("{}/{seed}/{w}x{h}.png", art_prefix()))
}

fn card(id: &str, kind: VideoEntryKind, title: &str, year: Option<u16>) -> VideoEntry {
    let playable = matches!(kind, VideoEntryKind::Movie | VideoEntryKind::Episode);
    VideoEntry {
        id: id.into(),
        kind,
        title: title.into(),
        year,
        series_id: None,
        series_title: None,
        season: None,
        episode: None,
        duration_ms: playable.then_some(6_120_000),
        position_ms: None,
        progress: None,
        played: false,
        favorite: false,
        unplayed_count: None,
        child_count: None,
        community_rating: Some(7.6),
        official_rating: Some("PG-13".into()),
        poster: art(id, 480, 720),
        landscape: art(&format!("{id}-wide"), 800, 450),
        backdrop: art(&format!("{id}-wide"), 1600, 900),
        logo: None,
        source: None,
        item: playable.then(|| MediaItemRef {
            id: id.into(),
            kind: MediaKind::Video,
            title: title.into(),
            artist: None,
            album: None,
            artwork_url: None,
            duration_ms: Some(6_120_000),
            series: None,
            index: None,
            played: false,
            starred: false,
            source: None,
        }),
    }
}

fn movie(index: usize) -> VideoEntry {
    let (id, title, year) = MOVIES[index % MOVIES.len()];
    card(id, VideoEntryKind::Movie, title, Some(year))
}

fn series(index: usize) -> VideoEntry {
    let (id, title, year) = SERIES[index % SERIES.len()];
    let mut entry = card(id, VideoEntryKind::Series, title, Some(year));
    entry.child_count = Some(2);
    entry.unplayed_count = Some(9);
    entry
}

fn episode(series: usize, season: u32, number: u32) -> VideoEntry {
    let (series_id, series_title, _) = SERIES[series % SERIES.len()];
    let name = EPISODES[(number as usize - 1) % EPISODES.len()];
    let id = format!("{series_id}-{season}-{number}");
    let mut entry = card(&id, VideoEntryKind::Episode, name, None);
    entry.series_id = Some(series_id.into());
    entry.series_title = Some(series_title.into());
    entry.season = Some(season);
    entry.episode = Some(number);
    entry.duration_ms = Some(2_640_000);
    entry.poster = entry.landscape.clone();
    if let Some(item) = &mut entry.item {
        item.series = Some(series_title.into());
        item.index = Some(number);
    }
    entry
}

fn watching(mut entry: VideoEntry, position_ms: u64) -> VideoEntry {
    entry.position_ms = Some(position_ms);
    let duration = entry.duration_ms.unwrap_or(1).max(1);
    #[allow(clippy::cast_precision_loss)]
    let progress = (position_ms as f64 / duration as f64) as f32;
    entry.progress = Some(progress);
    entry
}

pub(super) fn home() -> VideoHome {
    VideoHome {
        libraries: vec![
            VideoLibrary {
                id: "lib-movies".into(),
                title: "Movies".into(),
                kind: VideoLibraryKind::Movies,
                count: Some(MOVIES.len() as u32),
            },
            VideoLibrary {
                id: "lib-shows".into(),
                title: "Shows".into(),
                kind: VideoLibraryKind::Shows,
                count: Some(SERIES.len() as u32),
            },
        ],
        shelves: vec![
            VideoShelf {
                kind: VideoShelfKind::Resume,
                entries: vec![
                    watching(movie(2), 2_400_000),
                    watching(episode(0, 1, 3), 900_000),
                ],
            },
            VideoShelf {
                kind: VideoShelfKind::NextUp,
                entries: vec![episode(0, 1, 4), episode(1, 1, 2)],
            },
            VideoShelf {
                kind: VideoShelfKind::Latest,
                entries: (0..8).map(movie).collect(),
            },
        ],
        partial_failure: None,
    }
}

pub(super) fn wall(query: &VideoWallQuery) -> VideoWall {
    let entries: Vec<VideoEntry> = match query.parent.as_deref() {
        Some(parent) if parent.contains('-') => {
            let series = usize::from(parent.starts_with("s2"));
            (1..=6).map(|n| episode(series, 1, n)).collect()
        }
        _ if query.library.as_deref() == Some("lib-shows") => {
            (0..SERIES.len()).map(series).collect()
        }
        _ => (0..MOVIES.len()).map(movie).collect(),
    };
    VideoWall {
        total: Some(u32::try_from(entries.len()).unwrap_or(u32::MAX)),
        entries,
        cursor: None,
    }
}

fn track(index: u16, language: &str, label: &str, kind: TrackKind) -> MediaTrack {
    MediaTrack {
        index,
        language: language.into(),
        label: Some(label.into()),
        kind,
        selected: index <= 2,
    }
}

fn season(series: usize, number: u32) -> VideoEntry {
    let (series_id, series_title, _) = SERIES[series % SERIES.len()];
    let mut entry = card(
        &format!("{series_id}-{number}"),
        VideoEntryKind::Season,
        &format!("Season {number}"),
        None,
    );
    entry.series_id = Some(series_id.into());
    entry.series_title = Some(series_title.into());
    entry.season = Some(number);
    entry.child_count = Some(6);
    entry.unplayed_count = Some(6);
    entry
}

/// 剧的季（`s1-2`）和集（`s1-2-3`）按编号拆回来。
fn season_or_episode(id: &str) -> Option<VideoEntry> {
    let mut parts = id.split('-');
    let series = SERIES
        .iter()
        .position(|(sid, _, _)| Some(*sid) == parts.next())?;
    let number: u32 = parts.next()?.parse().ok()?;
    match parts.next() {
        None => Some(season(series, number)),
        Some(episode_number) => Some(episode(series, number, episode_number.parse().ok()?)),
    }
}

pub(super) fn detail(id: &str) -> VideoDetail {
    let is_series = SERIES.iter().position(|(sid, _, _)| *sid == id);
    let entry = match is_series {
        Some(index) => series(index),
        None => MOVIES
            .iter()
            .position(|(mid, _, _)| *mid == id)
            .map(movie)
            .or_else(|| season_or_episode(id))
            .unwrap_or_else(|| episode(0, 1, 1)),
    };
    let is_season = entry.kind == VideoEntryKind::Season;
    let is_episode = entry.kind == VideoEntryKind::Episode;
    let people = [
        ("Lin Wan", VideoRole::Actor, Some("Dr. Lin")),
        ("Zhou Yu", VideoRole::Actor, Some("Hai")),
        ("Chen Mo", VideoRole::Director, None),
    ]
    .iter()
    .map(|(name, role, character)| VideoPerson {
        name: (*name).into(),
        role: *role,
        character: character.map(Into::into),
        photo: art(name, 240, 240),
    })
    .collect();
    let series_index = SERIES
        .iter()
        .position(|(sid, _, _)| entry.series_id.as_deref() == Some(*sid));
    let children = match (is_series, series_index) {
        (Some(index), _) => (1..=2).map(|number| season(index, number)).collect(),
        (None, Some(index)) if is_season => {
            let number = entry.season.unwrap_or(1);
            let mut episodes: Vec<VideoEntry> =
                (1..=6).map(|n| episode(index, number, n)).collect();
            for watched in episodes.iter_mut().take(2) {
                watched.played = true;
            }
            episodes[2] = watching(episodes[2].clone(), 900_000);
            episodes
        }
        _ => Vec::new(),
    };
    let plain = is_series.is_none() && !is_season;
    VideoDetail {
        overview: Some(
            "A small seaside town and someone who wants to leave but can't let go. A few small things one summer slowly change everyone."
                .into(),
        ),
        tagline: None,
        genres: vec!["Drama".into(), "Family".into()],
        studios: vec!["Example Pictures".into()],
        premiered_at: Some(1_700_000_000_000),
        people,
        next_up: is_series.map(|index| watching(episode(index, 1, 3), 900_000)),
        children,
        audio: if !plain {
            Vec::new()
        } else {
            vec![
                track(1, "zho", "Mandarin · 5.1", TrackKind::Audio),
                track(3, "eng", "English · Stereo", TrackKind::Audio),
            ]
        },
        subtitles: if !plain {
            Vec::new()
        } else {
            vec![
                track(2, "zho", "Chinese (Simplified)", TrackKind::Subtitle),
                track(4, "eng", "English", TrackKind::Subtitle),
            ]
        },
        media: plain.then(|| VideoMediaInfo {
            width: Some(if is_episode { 1920 } else { 3840 }),
            height: Some(if is_episode { 1080 } else { 2160 }),
            hdr: !is_episode,
            video_codec: Some("hevc".into()),
            audio_codec: Some("eac3".into()),
            audio_channels: Some(6),
            container: Some("mkv".into()),
            size: Some(18_400_000_000),
        }),
        offline: offline(id),
        entry,
    }
}

/// 两部片子在“这台手机上”：一部下到一半，一部下完（路径是假的，点播放会放不了）。
pub(super) fn offline(id: &str) -> Option<OfflineState> {
    match id {
        "m2" => Some(OfflineState::Downloading {
            done: 7_700_000_000,
            total: Some(18_400_000_000),
        }),
        "m4" => Some(OfflineState::Ready {
            path: "/ui-fake/m4.mkv".into(),
        }),
        _ => None,
    }
}

pub(super) fn genres() -> Vec<String> {
    ["Drama", "Family", "Sci-Fi", "Mystery", "Documentary"]
        .map(String::from)
        .to_vec()
}

/// 假数据没有片源。
pub(super) fn no_stream() -> MediaFailure {
    MediaFailure {
        code: MediaFailureCode::TransportUnavailable,
        message: "ui-fake has no video stream".into(),
        retry_after_secs: None,
    }
}
