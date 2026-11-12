//! 第一个假提供方：每个用户一张有序表，查询直接按索引取区间。
//!
//! 上游里换提供方不该改变使用方的写法，所以这份实现只按契约存、查、删，不认得核心的工具。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use wonton_protocol::{PersonalDataFailure, RetentionPolicy};

pub(super) struct MapStore {
    users: BTreeMap<String, User>,
}

impl Default for MapStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MapStore {
    pub(super) fn new() -> Self {
        Self {
            users: BTreeMap::new(),
        }
    }
}

#[derive(Default)]
struct User {
    retention: BTreeMap<String, RetentionPolicy>,
    /// 键是（测量时间，设备），重传同一个点覆盖同一个键。
    location: BTreeMap<(u64, String), Value>,
    /// 类型 →（开始时刻，去重键）。
    health: BTreeMap<String, BTreeMap<(u64, String), Value>>,
    synced: BTreeMap<String, u64>,
}

impl super::serve::Store for MapStore {
    fn write_location(&mut self, user: &str, point: Value) -> super::serve::Reply {
        let point = super::serve::point(point)?;
        let user = super::serve::user(&mut self.users, user);
        if !super::serve::keeps(user.retention.get("location")) {
            return Ok(json!({"accepted": 0}));
        }
        user.location.insert((point.tst, point.tid), point.wire);
        Ok(json!({"accepted": 1}))
    }

    fn write_health(&mut self, user: &str, samples: Value) -> super::serve::Reply {
        let batch = super::serve::batch(samples)?;
        let user = super::serve::user(&mut self.users, user);
        let mut accepted = 0;
        let mut types: Vec<String> = Vec::new();
        for sample in batch.samples {
            if !types.contains(&sample.type_name) {
                types.push(sample.type_name.clone());
            }
            if !super::serve::keeps(user.retention.get(&sample.type_name)) {
                continue;
            }
            user.health
                .entry(sample.type_name)
                .or_default()
                .insert((sample.start_ms, sample.source_id), sample.wire);
            accepted += 1;
        }
        for type_name in types {
            if let Some(through) = batch.synced_through_ms {
                let current = user.synced.entry(type_name).or_insert(through);
                *current = (*current).max(through);
            }
        }
        Ok(json!({"accepted": accepted}))
    }

    fn query(&self, user: &str, ask: &super::serve::Ask) -> super::serve::Reply {
        let user = self.users.get(user);
        if ask.type_name == "location" {
            let points = user
                .into_iter()
                .flat_map(|user| user.location.iter())
                .filter(|((tst, _), _)| in_range(tst * 1000, ask))
                .map(|(_, wire)| wire.clone())
                .collect::<Vec<_>>();
            return Ok(json!({"kind": "location", "points": points}));
        }
        let map = user.and_then(|user| user.health.get(&ask.type_name));
        let synced = user.and_then(|user| user.synced.get(&ask.type_name));
        if map.is_none()
            && synced.is_none()
            && !super::serve::HEALTH_TYPES.contains(&ask.type_name.as_str())
        {
            return Err(super::serve::problem(
                PersonalDataFailure::NotSupported,
                &format!("不存类型 {}", ask.type_name),
            ));
        }
        let samples = map
            .into_iter()
            .flat_map(|map| map.iter())
            .filter(|((start, _), _)| in_range(*start, ask))
            .map(|(_, wire)| wire.clone())
            .collect::<Vec<_>>();
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
        if ask.type_name == "location" {
            let before = user.location.len();
            user.location
                .retain(|(tst, _), _| !in_range(tst * 1000, ask));
            return Ok(json!({"deleted": before - user.location.len()}));
        }
        let Some(map) = user.health.get_mut(&ask.type_name) else {
            return Ok(json!({"deleted": 0}));
        };
        let before = map.len();
        map.retain(|(start, _), _| !in_range(*start, ask));
        Ok(json!({"deleted": before - map.len()}))
    }

    fn export(&self, user: &str) -> super::serve::Reply {
        let user = self.users.get(user);
        let location = user
            .into_iter()
            .flat_map(|user| user.location.iter())
            .map(|(_, wire)| wire.clone())
            .collect::<Vec<_>>();
        let health = user
            .into_iter()
            .flat_map(|user| user.health.values())
            .flat_map(|map| map.iter())
            .map(|(_, wire)| wire.clone())
            .collect::<Vec<_>>();
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
            if type_name == "location" {
                let before = user.location.len();
                user.location.retain(|(tst, _), _| tst * 1000 >= cut);
                deleted += (before - user.location.len()) as u64;
                continue;
            }
            let Some(map) = user.health.get_mut(&type_name) else {
                continue;
            };
            let before = map.len();
            map.retain(|(start, _), _| *start >= cut);
            deleted += (before - map.len()) as u64;
        }
        Ok(json!({"deleted": deleted}))
    }
}

fn in_range(at: u64, ask: &super::serve::Ask) -> bool {
    at >= ask.from_ms && at <= ask.to_ms
}
