//! 假曲库的曲目表：8 位歌手、12 张专辑（一张双碟、一张有客座歌手）、4 张歌单（一张是家里别人的）。
//! 歌曲 ID 是 `<专辑 ID>-<第几首>`，从 1 数起；封面 ID 就是专辑、歌手、歌单的 ID。

pub(super) struct ArtistDef {
    pub(super) id: &'static str,
    pub(super) name: &'static str,
}

pub(super) struct AlbumDef {
    pub(super) id: &'static str,
    pub(super) title: &'static str,
    pub(super) artist: &'static str,
    pub(super) year: u16,
    pub(super) genre: &'static str,
    /// 两碟的专辑前一半在第 1 碟。
    pub(super) disc_titles: &'static [&'static str],
    pub(super) tracks: &'static [&'static str],
    /// 第几首（从 1 数）换成别的歌手署名。
    pub(super) guest: Option<(usize, &'static str)>,
}

pub(super) struct PlaylistDef {
    pub(super) id: &'static str,
    pub(super) name: &'static str,
    pub(super) owner: &'static str,
    pub(super) readonly: bool,
    pub(super) comment: Option<&'static str>,
    pub(super) songs: &'static [&'static str],
}

pub(super) const ARTISTS: &[ArtistDef] = &[
    ArtistDef {
        id: "ar-01",
        name: "Evening Breeze",
    },
    ArtistDef {
        id: "ar-02",
        name: "Lin Yi",
    },
    ArtistDef {
        id: "ar-03",
        name: "Aurora Lane",
    },
    ArtistDef {
        id: "ar-04",
        name: "Copper Fields",
    },
    ArtistDef {
        id: "ar-05",
        name: "Chen Xiaoyu",
    },
    ArtistDef {
        id: "ar-06",
        name: "Kaito Mori",
    },
    ArtistDef {
        id: "ar-07",
        name: "Orchestra Nova",
    },
    ArtistDef {
        id: "ar-08",
        name: "Bluestone",
    },
];

const NO_DISCS: &[&str] = &[];

pub(super) const ALBUMS: &[AlbumDef] = &[
    AlbumDef {
        id: "al-01",
        title: "Summer by the Sea",
        artist: "ar-01",
        year: 2025,
        genre: "Pop",
        disc_titles: NO_DISCS,
        tracks: &[
            "Summer by the Sea",
            "Tides",
            "Evening Wind",
            "White Shirt",
            "Lights Off",
            "Letter in August",
            "Slowly",
            "The Way Back",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-02",
        title: "Night Drive",
        artist: "ar-01",
        year: 2022,
        genre: "Pop",
        disc_titles: NO_DISCS,
        tracks: &[
            "Neon",
            "Last Train",
            "24th Floor",
            "Before the Rain Stops",
            "Night Drive",
            "After Dawn",
            "Sleepless",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-03",
        title: "Letters from the Hills",
        artist: "ar-02",
        year: 2024,
        genre: "Folk",
        disc_titles: NO_DISCS,
        tracks: &[
            "Letters from the Hills",
            "Chimney Smoke",
            "Winter in a Small Town",
            "The Old Tree",
            "Your Name",
            "Southbound Train",
            "Homeward",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-04",
        title: "Old Days",
        artist: "ar-02",
        year: 2019,
        genre: "Folk",
        disc_titles: NO_DISCS,
        tracks: &[
            "Old Days",
            "Grandma's Yard",
            "Cassette",
            "Dusk",
            "Falling Leaves",
            "Goodbye, Boy",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-05",
        title: "Northern Lights",
        artist: "ar-03",
        year: 2023,
        genre: "Electronic",
        disc_titles: NO_DISCS,
        tracks: &[
            "Polar Drift",
            "Northern Lights",
            "Static Bloom",
            "Afterglow",
            "Ice Hotel",
            "Signal Lost",
            "Midnight Sun",
            "Aurora",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-06",
        title: "Glass Cities",
        artist: "ar-03",
        year: 2025,
        genre: "Electronic",
        disc_titles: NO_DISCS,
        tracks: &[
            "Glass Cities",
            "Neon Rain",
            "Overpass",
            "Satellite Heart",
            "Low Orbit",
            "Reflections",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-07",
        title: "Slow Rivers",
        artist: "ar-04",
        year: 2021,
        genre: "Indie",
        disc_titles: NO_DISCS,
        tracks: &[
            "Slow Rivers",
            "Paper Boats",
            "Harbor Lights",
            "Wildflower",
            "Long Way Home",
            "Copper Sky",
            "Evergreen",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-08",
        title: "Rainy Season",
        artist: "ar-05",
        year: 2018,
        genre: "Pop",
        disc_titles: NO_DISCS,
        tracks: &[
            "Rainy Season",
            "Umbrella",
            "Outside the Window",
            "You Said",
            "The Seventh Day",
            "After the Sun",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-09",
        title: "Tokyo Tapes",
        artist: "ar-06",
        year: 2023,
        genre: "Jazz",
        disc_titles: NO_DISCS,
        tracks: &[
            "Shibuya 2AM",
            "Rainy Kissa",
            "Late Train",
            "Vinyl Café",
            "Blue Hour",
            "Neon Koi",
            "Home by Dawn",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-10",
        title: "Four Seasons Reimagined",
        artist: "ar-07",
        year: 2017,
        genre: "Classical",
        disc_titles: &["Spring & Summer", "Autumn & Winter"],
        tracks: &[
            "Spring I. Allegro",
            "Spring II. Largo",
            "Spring III. Danza",
            "Summer I. Languore",
            "Summer II. Adagio",
            "Summer III. Presto",
            "Autumn I. Allegro",
            "Autumn II. Adagio",
            "Autumn III. La caccia",
            "Winter I. Allegro non molto",
            "Winter II. Largo",
            "Winter III. Allegro",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-11",
        title: "Echo",
        artist: "ar-08",
        year: 2026,
        genre: "Rock",
        disc_titles: NO_DISCS,
        tracks: &[
            "Echo",
            "Broken Bridge",
            "Burn",
            "No Man's Land",
            "Last Stop",
            "Light",
        ],
        guest: None,
    },
    AlbumDef {
        id: "al-12",
        title: "Live at the Lakeside",
        artist: "ar-04",
        year: 2024,
        genre: "Indie",
        disc_titles: NO_DISCS,
        tracks: &[
            "Intro",
            "Slow Rivers (Live)",
            "Wildflower (Live)",
            "Letters from the Hills (Live)",
            "Encore",
        ],
        guest: Some((4, "Copper Fields & Lin Yi")),
    },
];

pub(super) const PLAYLISTS: &[PlaylistDef] = &[
    PlaylistDef {
        id: "pl-01",
        name: "Commute",
        owner: "owner",
        readonly: false,
        comment: None,
        songs: &[
            "al-02-1", "al-02-2", "al-05-2", "al-09-3", "al-06-2", "al-01-3", "al-07-5",
        ],
    },
    PlaylistDef {
        id: "pl-02",
        name: "Weekend Morning",
        owner: "owner",
        readonly: false,
        comment: None,
        songs: &[
            "al-03-1", "al-04-2", "al-07-2", "al-09-4", "al-08-6", "al-12-4",
        ],
    },
    PlaylistDef {
        id: "pl-03",
        name: "Running",
        owner: "owner",
        readonly: false,
        comment: None,
        songs: &[
            "al-11-3", "al-05-1", "al-06-1", "al-11-1", "al-02-5", "al-05-7",
        ],
    },
    PlaylistDef {
        id: "pl-04",
        name: "Golden Oldies",
        owner: "Mom",
        readonly: true,
        comment: Some("For cooking on weekends"),
        songs: &["al-04-1", "al-04-3", "al-08-1", "al-08-2", "al-03-3"],
    },
];

/// 最近播放、常听的专辑。
pub(super) const RECENT: &[&str] = &["al-05", "al-01", "al-09", "al-03", "al-11", "al-07"];
pub(super) const FREQUENT: &[&str] = &[
    "al-01", "al-03", "al-08", "al-05", "al-02", "al-09", "al-10",
];

/// 一开始就收藏了的。
pub(super) const STARRED: &[&str] = &["al-01", "al-05", "al-03-1", "al-09-1", "al-07-1", "ar-02"];

/// 有同步歌词的一首。
pub(super) const SYNCED_LYRICS: (&str, &[(u64, &str)]) = (
    "al-01-1",
    &[
        (0, "(Intro)"),
        (12_000, "The wind comes in from the sea"),
        (17_500, "And tangles up your hair"),
        (23_000, "We walk along the long sea wall"),
        (29_000, "And promise not to talk of tomorrow"),
        (36_000, "Summer by the Sea"),
        (41_500, "Like a letter left unfinished"),
        (47_000, "When the tide goes out"),
        (52_500, "I'll read it to you"),
    ],
);

/// 只有文字、不带时间的歌词。
pub(super) const PLAIN_LYRICS: (&str, &[&str]) = (
    "al-03-1",
    &[
        "A letter came from over the hills",
        "Saying the snow came early this year",
        "The ones sitting by the fire",
        "Are still waiting for you to come home",
    ],
);
