use iroh::EndpointId;
use serde::{Deserialize, Serialize};
use std::fmt::Write;
use wonton_protocol::{JoinRequest, MemberStatus};

use super::{MemberEvent, Members, MembersError, code_hash};
use crate::device::Devices;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
enum Decision {
    Pending,
    Approved,
    Rejected,
    RejectionSeen,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct StoredJoin {
    pub(super) request: JoinRequest,
    endpoint: EndpointId,
    code_hash: String,
    expires_at_ms: u64,
    decision: Decision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinCheck {
    Unknown,
    Expired,
    Pending(String),
    Paired,
    Rejected,
    CodeUsed,
}

impl Members {
    pub(crate) fn pending_endpoint(&self, endpoint: EndpointId, username: &str) -> bool {
        self.lock().joins.iter().any(|join| {
            join.endpoint == endpoint
                && join.request.username == username
                && join.decision == Decision::Pending
        })
    }

    /// 中断在登记设备和保存批准之间时，重启不能让待批准的设备进入白名单。
    pub fn recover_pending_devices(&self, devices: &Devices) -> Result<(), MembersError> {
        let stored = self.lock();
        for join in &stored.joins {
            if join.decision == Decision::Pending
                && devices.get(&join.endpoint).is_some_and(|device| {
                    device.user.as_deref() == Some(join.request.username.as_str())
                })
            {
                devices.remove(&join.endpoint)?;
            }
        }
        Ok(())
    }

    pub fn join_requests(&self) -> Vec<JoinRequest> {
        let stored = self.lock();
        let now = (self.now_ms)();
        stored
            .joins
            .iter()
            .filter(|join| {
                join.decision == Decision::Pending
                    && join.expires_at_ms > now
                    && stored.members.iter().any(|member| {
                        member.username == join.request.username
                            && member.status == MemberStatus::Invited
                            && member.pairing_code_hash.as_deref() == Some(&join.code_hash)
                    })
            })
            .map(|join| join.request.clone())
            .collect()
    }

    /// 校验、占用邀请码和记请求共用成员锁；另一台 endpoint 不能抢走已提交的请求。
    pub fn request_join(
        &self,
        secret_hex: &str,
        endpoint: EndpointId,
        device_name: &str,
        devices: &Devices,
    ) -> Result<JoinCheck, MembersError> {
        let hash = code_hash(secret_hex);
        let now = (self.now_ms)();
        let mut stored = self.lock();
        let Some(member) = stored
            .members
            .iter()
            .find(|member| member.pairing_code_hash.as_deref() == Some(&hash))
        else {
            return Ok(JoinCheck::Unknown);
        };
        let expires_at_ms = member.pairing_code_expires_at_ms.unwrap_or(0);
        if expires_at_ms <= now || member.status == MemberStatus::Removed {
            return Ok(JoinCheck::Expired);
        }
        if member.status == MemberStatus::Other {
            return Err(MembersError::Invalid("成员状态无法识别".to_owned()));
        }
        if let Some(index) = stored.joins.iter().position(|join| join.code_hash == hash) {
            let join = &stored.joins[index];
            if join.endpoint != endpoint {
                return Ok(JoinCheck::CodeUsed);
            }
            return match join.decision {
                Decision::Pending => Ok(JoinCheck::Pending(member.role_id.clone())),
                Decision::Approved => Ok(
                    if devices.get(&endpoint).is_some_and(|device| {
                        device.user.as_deref() == Some(member.username.as_str())
                    }) {
                        JoinCheck::Paired
                    } else {
                        JoinCheck::CodeUsed
                    },
                ),
                Decision::Rejected => {
                    let mut next = stored.clone();
                    next.joins[index].decision = Decision::RejectionSeen;
                    self.commit(&mut stored, next)?;
                    Ok(JoinCheck::Rejected)
                }
                Decision::RejectionSeen => Ok(JoinCheck::CodeUsed),
            };
        }
        if devices.contains(&endpoint) {
            return Ok(JoinCheck::CodeUsed);
        }
        let mut bytes = [0; 16];
        getrandom::fill(&mut bytes).expect("系统随机数不可用");
        let request = JoinRequest {
            id: request_id(bytes),
            username: member.username.clone(),
            display_name: member.display_name.clone(),
            role_id: member.role_id.clone(),
            device_name: device_name.to_owned(),
            requested_at_ms: now,
        };
        let joined = member.status == MemberStatus::Joined;
        let check = if joined {
            JoinCheck::Paired
        } else {
            JoinCheck::Pending(member.role_id.clone())
        };
        let mut next = stored.clone();
        next.joins.push(StoredJoin {
            request,
            endpoint,
            code_hash: hash,
            expires_at_ms,
            decision: if joined {
                Decision::Approved
            } else {
                Decision::Pending
            },
        });
        // 已加入成员的管理员重发码也直接配对；新成员必须等批准。
        if joined {
            let request = &next.joins.last().expect("刚加入记录").request;
            register(devices, endpoint, request)?;
        }
        if let Err(error) = self.commit(&mut stored, next) {
            if joined {
                devices.remove(&endpoint)?;
            }
            return Err(error);
        }
        Ok(check)
    }

    pub fn approve_join(&self, id: &str, devices: &Devices) -> Result<(), MembersError> {
        self.decide_join(id, Some(devices))
    }

    pub fn reject_join(&self, id: &str) -> Result<(), MembersError> {
        self.decide_join(id, None)
    }

    fn decide_join(&self, id: &str, devices: Option<&Devices>) -> Result<(), MembersError> {
        let mut stored = self.lock();
        let now = (self.now_ms)();
        let index = stored
            .joins
            .iter()
            .position(|join| {
                join.request.id == id
                    && join.decision == Decision::Pending
                    && join.expires_at_ms > now
                    && stored.members.iter().any(|member| {
                        member.username == join.request.username
                            && member.status == MemberStatus::Invited
                            && member.pairing_code_hash.as_deref() == Some(&join.code_hash)
                    })
            })
            .ok_or_else(|| MembersError::NotFound(id.to_owned()))?;
        let mut next = stored.clone();
        let join = &mut next.joins[index];
        let username = join.request.username.clone();
        let endpoint = join.endpoint;
        if let Some(devices) = devices {
            register(devices, endpoint, &join.request)?;
            join.decision = Decision::Approved;
            let member = next
                .members
                .iter_mut()
                .find(|member| member.username == username)
                .expect("请求对应成员已校验");
            member.status = MemberStatus::Joined;
        } else {
            join.decision = Decision::Rejected;
        }
        if let Err(error) = self.commit(&mut stored, next) {
            if let Some(devices) = devices {
                devices.remove(&endpoint)?;
            }
            return Err(error);
        }
        drop(stored);
        if devices.is_some() {
            let _ = self.events.send(MemberEvent::Joined { username });
        }
        Ok(())
    }
}

fn register(
    devices: &Devices,
    endpoint: EndpointId,
    request: &JoinRequest,
) -> Result<(), MembersError> {
    let added = devices
        .add_if(
            endpoint,
            Some(request.device_name.clone()),
            Some(request.username.clone()),
            |registered| !registered.contains_key(&endpoint),
        )
        .map_err(MembersError::Device)?;
    if added != Some(true) {
        return Err(MembersError::DeviceRegistered);
    }
    Ok(())
}

fn request_id(bytes: [u8; 16]) -> String {
    let mut id = String::with_capacity(32);
    for byte in bytes {
        write!(id, "{byte:02x}").expect("写入 String 不会失败");
    }
    id
}
