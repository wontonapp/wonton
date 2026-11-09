use super::*;

struct Link {
    quality: Quality,
    path: PathInfo,
    now: Instant,
}

impl Link {
    fn new() -> Self {
        let mut link = Self {
            quality: Quality::default(),
            path: PathInfo {
                connection_id: 1,
                kind: PathKind::Direct,
                remote: TransportAddr::Ip("127.0.0.1:1".parse().unwrap()),
                rtt: Duration::from_millis(20),
                sent_packets: 0,
                lost_packets: 0,
                sent_bytes: 0,
                lost_bytes: 0,
            },
            now: Instant::now(),
        };
        assert_eq!(
            link.quality.observe(link.now, &link.path),
            Reading::default()
        );
        link
    }

    fn step(&mut self, elapsed: Duration, sent: u64, lost: u64, rtt_ms: u64) -> Reading {
        self.now += elapsed;
        self.path.sent_packets += sent;
        self.path.lost_packets += lost;
        self.path.sent_bytes += sent * 1_000;
        self.path.lost_bytes += lost * 1_000;
        self.path.rtt = Duration::from_millis(rtt_ms);
        self.quality.observe(self.now, &self.path)
    }

    fn seconds(&mut self, seconds: u64, lost: u64, rtt_ms: u64) -> Reading {
        let mut reading = Reading::default();
        for _ in 0..seconds {
            reading = self.step(INTERVAL, 100, lost, rtt_ms);
        }
        reading
    }
}

#[test]
fn forty_nine_packets_are_unknown_and_fifty_publish_metrics() {
    let mut link = Link::new();
    assert_eq!(link.step(INTERVAL, 49, 0, 20), Reading::default());
    let enough = link.step(INTERVAL, 1, 0, 20);
    assert_eq!(enough.quality, ConnectionQuality::Unknown);
    assert_eq!(enough.loss_per_mille, Some(0));
    assert_eq!(enough.rtt_ms, Some(20));
}

#[test]
fn good_requires_thirty_complete_seconds() {
    let mut link = Link::new();
    link.seconds(1, 0, 20);
    assert_eq!(link.seconds(29, 0, 20).quality, ConnectionQuality::Unknown);
    assert_eq!(link.seconds(1, 0, 20).quality, ConnectionQuality::Good);
}

#[test]
fn five_percent_loss_requires_ten_complete_seconds() {
    let mut link = Link::new();
    link.seconds(1, 5, 20);
    assert_eq!(link.seconds(9, 5, 20).quality, ConnectionQuality::Unknown);
    let poor = link.seconds(1, 5, 20);
    assert_eq!(poor.quality, ConnectionQuality::Poor);
    assert_eq!(poor.loss_per_mille, Some(50));
}

#[test]
fn slow_direct_and_relay_paths_use_the_same_threshold() {
    for kind in [PathKind::Direct, PathKind::Relay] {
        let mut link = Link::new();
        link.path.kind = kind;
        link.quality.observe(link.now, &link.path);
        link.seconds(1, 0, 800);
        assert_eq!(link.seconds(9, 0, 800).quality, ConnectionQuality::Unknown);
        assert_eq!(link.seconds(1, 0, 800).quality, ConnectionQuality::Poor);
    }
}

#[test]
fn three_seconds_of_loss_do_not_flip_after_the_burst_ends() {
    let mut link = Link::new();
    assert_eq!(link.seconds(31, 0, 20).quality, ConnectionQuality::Good);
    let spike = link.seconds(3, 100, 20);
    assert!(spike.loss_per_mille.unwrap() >= 50);
    assert_eq!(spike.quality, ConnectionQuality::Good);
    for _ in 0..40 {
        assert_eq!(link.seconds(1, 0, 20).quality, ConnectionQuality::Good);
    }
}

#[test]
fn recovery_waits_for_a_clean_window_and_thirty_seconds() {
    let mut link = Link::new();
    assert_eq!(link.seconds(12, 20, 20).quality, ConnectionQuality::Poor);
    for _ in 0..30 {
        let reading = link.seconds(1, 0, 20);
        assert_eq!(reading.quality, ConnectionQuality::Poor);
        if reading.loss_per_mille.unwrap() < 20 {
            let before = link.step(Duration::from_secs(29), 100, 0, 20);
            assert_eq!(before.quality, ConnectionQuality::Poor);
            assert_eq!(link.seconds(1, 0, 20).quality, ConnectionQuality::Good);
            return;
        }
    }
    panic!("old loss did not expire from the thirty-second window");
}

#[test]
fn gray_zone_keeps_the_previous_judgement() {
    let mut link = Link::new();
    assert_eq!(link.seconds(31, 0, 20).quality, ConnectionQuality::Good);
    let gray = link.seconds(35, 3, 600);
    assert_eq!(gray.quality, ConnectionQuality::Good);
    assert_eq!(gray.loss_per_mille, Some(30));
    assert_eq!(gray.rtt_ms, Some(600));
}

#[test]
fn reconnect_and_path_switch_discard_lifetime_totals() {
    let mut link = Link::new();
    assert_eq!(link.seconds(31, 0, 20).quality, ConnectionQuality::Good);
    link.path.connection_id += 1;
    link.path.sent_packets += 10_000;
    link.path.sent_bytes += 10_000_000;
    assert_eq!(
        link.quality.observe(link.now, &link.path),
        Reading::default()
    );
    link.seconds(31, 0, 20);
    link.path.remote = TransportAddr::Ip("127.0.0.1:2".parse().unwrap());
    assert_eq!(
        link.quality.observe(link.now, &link.path),
        Reading::default()
    );
}

#[test]
fn ack_only_packets_do_not_dilute_lost_data() {
    let mut link = Link::new();
    for _ in 0..11 {
        link.path.sent_packets += 10_000;
        let reading = link.step(INTERVAL, 50, 10, 20);
        assert_eq!(reading.loss_per_mille, Some(200));
    }
    assert_eq!(link.quality.reading.quality, ConnectionQuality::Poor);
}

#[test]
fn idle_and_sampling_gaps_return_to_unknown() {
    let mut link = Link::new();
    link.seconds(31, 0, 20);
    assert_eq!(link.step(WINDOW, 0, 0, 20), Reading::default());
    assert_eq!(
        link.step(WINDOW + INTERVAL, 10_000, 2_000, 20),
        Reading::default()
    );
}

#[test]
fn clean_seconds_do_not_hide_sustained_loss_in_sparse_traffic() {
    let mut link = Link::new();
    for second in 1..=40 {
        let lost = u64::from(second % 3 == 1);
        link.step(INTERVAL, 2, lost, 20);
    }
    assert_eq!(link.quality.reading.quality, ConnectionQuality::Poor);
}
