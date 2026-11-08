//! QUIC DATAGRAM 内容：`flow:quic_varint | flags:u8 | [packet:u64be] | payload`。
//!
//! 流向为 0..=16383，编码用最短的 1/2 字节 QUIC 整数（63/64 是分界）；解码接受这两种宽度，
//! 拒绝 4/8 字节。flags 为 0（完整包）、1（两片中的第一片）、2（第二片）；其他值非法。
//! 仅分片携带 8 字节包编号，每片都携带，头最多 11 字节，完整包头最多 3 字节。
//! 空 UDP 包合法，空分片非法。每个包只能是完整包或恰好两片，不排序不同包、不补齐缺失片。
//! 包编号按 `(连接、方向、flow)` 隔离；发送方不得在该作用域内复用编号，包括重建同号 flow 后。
//! u64 耗尽须结束连接；接收方必须丢弃已经完成/过期包的迟到片并限制保存期限，不能只按片序号配对。
//! 上层须按 `(flow, packet_id)` 保存片，并在超时或完成后淘汰；相同键的重复片不得覆盖其他片。
//! 这些生命周期策略由调用方负责，本模块只验证一对片的 flow、包编号和片序号。
//! 每次 split/encode 都须传入发送前现取的 `max_datagram_size()`，不能缓存路径 MTU。

use super::CodecError;

/// 数据报和 UDP 成功应答共用的有界 QUIC 流向编号。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FlowId(u16);

impl FlowId {
    pub const MAX: u16 = 16383;

    pub fn new(value: u16) -> Result<Self, CodecError> {
        if value > Self::MAX {
            return Err(CodecError::InvalidFlowId);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> u16 {
        self.0
    }

    pub fn encoded_len(self) -> usize {
        if self.0 < 64 { 1 } else { 2 }
    }

    pub fn encode(self, output: &mut [u8]) -> Result<usize, CodecError> {
        let len = self.encoded_len();
        if output.len() < len {
            return Err(CodecError::BufferTooSmall);
        }
        if len == 1 {
            output[0] = self.0 as u8;
        } else {
            output[..2].copy_from_slice(&(self.0 | 0x4000).to_be_bytes());
        }
        Ok(len)
    }

    pub fn decode(input: &[u8]) -> Result<(Self, usize), CodecError> {
        let first = *input.first().ok_or(CodecError::Truncated)?;
        match first >> 6 {
            0 => Ok((Self(u16::from(first)), 1)),
            1 => {
                let second = *input.get(1).ok_or(CodecError::Truncated)?;
                Ok((Self(u16::from_be_bytes([first & 0x3f, second])), 2))
            }
            _ => Err(CodecError::InvalidFlowId),
        }
    }
}

/// 包内两片的位置，不表示不同 UDP 包之间的顺序。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FragmentIndex {
    First,
    Second,
}

/// 同一 flow 内不能复用的包编号及片序号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fragment {
    pub packet_id: u64,
    pub index: FragmentIndex,
}

/// 借用原始载荷的数据报帧；`None` 分片信息表示完整 UDP 包。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame<'a> {
    pub flow: FlowId,
    pub fragment: Option<Fragment>,
    pub payload: &'a [u8],
}

impl<'a> Frame<'a> {
    pub fn header_len(self) -> usize {
        self.flow.encoded_len() + 1 + if self.fragment.is_some() { 8 } else { 0 }
    }

    /// 只写头；载荷仍借用输入，可供调用方使用分散写入而无需复制。
    /// 即使只写头也按完整帧长度检查本次数据报上限。
    pub fn encode_header(
        self,
        output: &mut [u8],
        max_datagram_size: usize,
    ) -> Result<usize, CodecError> {
        self.validate()?;
        let header_len = self.header_len();
        let budget = max_datagram_size
            .checked_sub(header_len)
            .ok_or(CodecError::DatagramTooSmall)?;
        if self.payload.len() > budget {
            return Err(CodecError::PacketTooLarge);
        }
        if output.len() < header_len {
            return Err(CodecError::BufferTooSmall);
        }
        let used = self.flow.encode(output)?;
        output[used] = match self.fragment {
            None => 0,
            Some(Fragment {
                index: FragmentIndex::First,
                ..
            }) => 1,
            Some(Fragment {
                index: FragmentIndex::Second,
                ..
            }) => 2,
        };
        if let Some(fragment) = self.fragment {
            output[used + 1..header_len].copy_from_slice(&fragment.packet_id.to_be_bytes());
        }
        Ok(header_len)
    }

    /// 写入调用方缓冲区供需要连续字节的传输 API 使用，不分配内存。
    pub fn encode(self, output: &mut [u8], max_datagram_size: usize) -> Result<usize, CodecError> {
        let len = self
            .header_len()
            .checked_add(self.payload.len())
            .ok_or(CodecError::PacketTooLarge)?;
        if output.len() < len {
            return Err(CodecError::BufferTooSmall);
        }
        let header_len = self.encode_header(output, max_datagram_size)?;
        output[header_len..len].copy_from_slice(self.payload);
        Ok(len)
    }

    pub fn decode(input: &'a [u8]) -> Result<Self, CodecError> {
        let (flow, used) = FlowId::decode(input)?;
        let flags = *input.get(used).ok_or(CodecError::Truncated)?;
        let mut header_len = used + 1;
        let fragment = match flags {
            0 => None,
            1 | 2 => {
                let bytes = input
                    .get(header_len..header_len + 8)
                    .ok_or(CodecError::Truncated)?;
                let mut id = [0; 8];
                id.copy_from_slice(bytes);
                header_len += 8;
                Some(Fragment {
                    packet_id: u64::from_be_bytes(id),
                    index: if flags == 1 {
                        FragmentIndex::First
                    } else {
                        FragmentIndex::Second
                    },
                })
            }
            _ => return Err(CodecError::InvalidFragment),
        };
        let frame = Self {
            flow,
            fragment,
            payload: &input[header_len..],
        };
        frame.validate()?;
        Ok(frame)
    }

    fn validate(self) -> Result<(), CodecError> {
        if self.fragment.is_some() && self.payload.is_empty() {
            return Err(CodecError::InvalidFragment);
        }
        Ok(())
    }
}

/// 一个完整包或两片，所有载荷均借用原输入。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Packet<'a> {
    Whole(Frame<'a>),
    Pair([Frame<'a>; 2]),
}

/// 先扣除各片实际头长再分片；完整包不加包编号且不复制载荷。
/// `packet_id` 只在需要分片时使用，唯一性约束见模块文档。
pub fn split(
    flow: FlowId,
    packet_id: u64,
    payload: &[u8],
    max_datagram_size: usize,
) -> Result<Packet<'_>, CodecError> {
    let whole = Frame {
        flow,
        fragment: None,
        payload,
    };
    let whole_budget = max_datagram_size
        .checked_sub(whole.header_len())
        .ok_or(CodecError::DatagramTooSmall)?;
    if payload.len() <= whole_budget {
        return Ok(Packet::Whole(whole));
    }
    let budget = max_datagram_size
        .checked_sub(whole.header_len() + 8)
        .filter(|budget| *budget > 0)
        .ok_or(CodecError::DatagramTooSmall)?;
    if payload.len().saturating_sub(budget) > budget {
        return Err(CodecError::PacketTooLarge);
    }
    let (first, second) = payload.split_at(budget);
    Ok(Packet::Pair([
        Frame {
            flow,
            fragment: Some(Fragment {
                packet_id,
                index: FragmentIndex::First,
            }),
            payload: first,
        },
        Frame {
            flow,
            fragment: Some(Fragment {
                packet_id,
                index: FragmentIndex::Second,
            }),
            payload: second,
        },
    ]))
}

/// 缺任一片返回 `None`，完整包直接借用其载荷；两片只在校验相符后复制进调用方缓冲。
/// 接受乱序的一对，拒绝重复片和跨 flow/包拼接，不保存任何重组状态。
pub fn join<'a>(
    first: Option<Frame<'a>>,
    second: Option<Frame<'a>>,
    output: &'a mut [u8],
) -> Result<Option<&'a [u8]>, CodecError> {
    let (first, second) = match (first, second) {
        (None, None) => return Ok(None),
        (Some(frame), None) | (None, Some(frame)) => {
            frame.validate()?;
            return Ok(if frame.fragment.is_none() {
                Some(frame.payload)
            } else {
                None
            });
        }
        (Some(first), Some(second)) => (first, second),
    };
    first.validate()?;
    second.validate()?;
    let (Some(a), Some(b)) = (first.fragment, second.fragment) else {
        return Err(CodecError::MismatchedFragments);
    };
    if first.flow != second.flow || a.packet_id != b.packet_id || a.index == b.index {
        return Err(CodecError::MismatchedFragments);
    }
    let (first, second) = if a.index == FragmentIndex::First {
        (first, second)
    } else {
        (second, first)
    };
    let len = first
        .payload
        .len()
        .checked_add(second.payload.len())
        .ok_or(CodecError::PacketTooLarge)?;
    if output.len() < len {
        return Err(CodecError::BufferTooSmall);
    }
    output[..first.payload.len()].copy_from_slice(first.payload);
    output[first.payload.len()..len].copy_from_slice(second.payload);
    Ok(Some(&output[..len]))
}

#[cfg(test)]
mod tests;
