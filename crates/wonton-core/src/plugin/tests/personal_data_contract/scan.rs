//! 第二个假提供方：每个用户一条流水，查询和清理都从头扫。存法和第一个不一样，契约一样。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use wonton_protocol::{PersonalDataFailure, RetentionPolicy};

pub(super) struct ScanStore {
    users: BTreeMap<String, User>,
}

impl Default for ScanStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ScanStore {
    pub(super) fn new() -> Self {
        Self {
            users: BTreeMap::new(),
        }
    }
}

#[derive(Default)]
struct User {
    retention: BTreeMap<String, RetentionPolicy>,
    rows: Vec<Row>,
    synced: BTreeMap<String, u64>,
}

enum Row {
    Location {
        tst: u64,
        tid: String,
        wire: Value,
    },
    Health {
        type_name: String,
        start_ms: u64,
        source_id: String,
        wire: Value,
    },
}

impl Row {
    fn start_ms(&self) -> u64 {
        match self {
            Self::Location { tst, .. } => tst * 1000,
            Self::Health { start_ms, .. } => *start_ms,
        }
    }

    fn wire(&self) -> &Value {
        match self {
            Self::Location { wire, .. } | Self::Health { wire, .. } => wire,
        }
    }
}

impl super::serve::Store for ScanStore {
    fn write_location(&mut self, user: &str, point: Value) -> super::serve::Reply {
        let point = super::serve::point(point)?;
        let user = super::serve::user(&mut self.users, user);
        if !super::serve::keeps(user.retention.get("location")) {
            return Ok(json!({"accepted": 0}));
        }
        // 重传同一个（时间，设备）就替换，不叠加。
        user.rows.retain(|row| match row {
            Row::Location { tst, tid, .. } => *tst != point.tst || *tid != point.tid,
            Row::Health { .. } => true,
        });
        user.rows.push(Row::Location {
            tst: point.tst,
            tid: point.tid,
            wire: point.wire,
        });
        Ok(json!({"accepted": 1}))
    }

    fn write_health(&mut self, user: &str, samples: Value) -> super::serve::Reply {
        let batch = super::serve::batch(samples)?;
        let user = super::serve::user(&mut self.users, user);
        let mut accepted = 0;
        let mut types: Vec<String> = Vec::new();
        for sample in batch.samples {
            types.push(sample.type_name.clone());
            if !super::serve::keeps(user.retention.get(&sample.type_name)) {
                continue;
            }
            user.rows.retain(|row| match row {
                Row::Health {
                    type_name,
                    source_id,
                    ..
                } => *type_name != sample.type_name || *source_id != sample.source_id,
                Row::Location { .. } => true,
            });
            user.rows.push(Row::Health {
                type_name: sample.type_name,
                start_ms: sample.start_ms,
                source_id: sample.source_id,
                wire: sample.wire,
            });
            accepted += 1;
        }
        if let Some(through) = batch.synced_through_ms {
            for type_name in types {
                let current = user.synced.entry(type_name).or_insert(through);
                *current = (*current).max(through);
            }
        }
        Ok(json!({"accepted": accepted}))
    }

    fn query(&self, user: &str, ask: &super::serve::Ask) -> super::serve::Reply {
        let user = self.users.get(user);
        if ask.type_name == "location" {
            let mut points: Vec<&Value> = self
                .filtered(user, |row| matches!(row, Row::Location { .. }), ask)
                .map(Row::wire)
                .collect();
            points.sort_by_key(|wire| wire["tst"].as_u64());
            return Ok(json!({"kind": "location", "points": points}));
        }
        let mut samples: Vec<&Value> = self
            .filtered(
                user,
                |row| matches!(row, Row::Health { type_name, .. } if *type_name == ask.type_name),
                ask,
            )
            .map(Row::wire)
            .collect();
        if samples.is_empty()
            && !super::serve::HEALTH_TYPES.contains(&ask.type_name.as_str())
            && user.is_none_or(|user| !user.synced.contains_key(&ask.type_name))
        {
            return Err(super::serve::problem(
                PersonalDataFailure::NotSupported,
                &format!("不存类型 {}", ask.type_name),
            ));
        }
        samples.sort_by_key(|wire| wire["start_ms"].as_u64());
        let synced = user.and_then(|user| user.synced.get(&ask.type_name));
        Ok(json!({
            "kind": "health",
            "samples": samples,
            "synced_through_ms": synced,
        }))
    }

    fn delete(&mut self, user: &str, ask: &super::serve::Ask) -> super::serve::Reply {
        let Some(user) = self.users.get_mut(user) else {
            return Ok(json!({"deleted": 0}));
        };
        let before = user.rows.len();
        user.rows.retain(|row| !self::hits(row, ask));
        Ok(json!({"deleted": before - user.rows.len()}))
    }

    fn export(&self, user: &str) -> super::serve::Reply {
        let Some(user) = self.users.get(user) else {
            return Ok(json!({
                "location": [],
                "health": [],
                "exported_at_ms": super::serve::now_ms(),
            }));
        };
        let mut rows: Vec<&Row> = user.rows.iter().collect();
        rows.sort_by_key(|row| row.start_ms());
        let (mut location, mut health) = (Vec::new(), Vec::new());
        for row in rows {
            match row {
                Row::Location { wire, .. } => location.push(wire),
                Row::Health { wire, .. } => health.push(wire),
            }
        }
        Ok(json!({
            "location": location,
            "health": health,
            "exported_at_ms": super::serve::now_ms(),
        }))
    }

    fn cleanup(&mut self, user: &str, request: Value) -> super::serve::Reply {
        let policies = super::serve::policies(&request)?;
        let user = super::serve::user(&mut self.users, user);
        let mut deleted = 0u64;
        for (type_name, policy) in policies {
            user.retention.insert(type_name.clone(), policy);
            let Some(cut) = super::serve::cutoff(&policy) else {
                continue;
            };
            let before = user.rows.len();
            user.rows.retain(|row| {
                let matches_type = match row {
                    Row::Location { .. } => type_name == "location",
                    Row::Health {
                        type_name: kind, ..
                    } => *kind == type_name,
                };
                !matches_type || row.start_ms() >= cut
            });
            deleted += (before - user.rows.len()) as u64;
        }
        Ok(json!({"deleted": deleted}))
    }
}

impl ScanStore {
    fn filtered<'a>(
        &'a self,
        user: Option<&'a User>,
        keep: impl Fn(&Row) -> bool,
        ask: &'a super::serve::Ask,
    ) -> impl Iterator<Item = &'a Row> {
        user.into_iter()
            .flat_map(|user| user.rows.iter())
            .filter(move |row| keep(row) && hits(row, ask))
    }
}

fn hits(row: &Row, ask: &super::serve::Ask) -> bool {
    let matches_type = match row {
        Row::Location { .. } => ask.type_name == "location",
        Row::Health { type_name, .. } => *type_name == ask.type_name,
    };
    matches_type && row.start_ms() >= ask.from_ms && row.start_ms() <= ask.to_ms
}
