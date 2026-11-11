use std::time::Duration;

use iroh::endpoint::{RecvStream, SendStream};
use serde::{Serialize, de::DeserializeOwned};

use super::ShareError;

pub(super) const FORWARD: u8 = 0;
pub(super) const CONTROL: u8 = 1;
pub(super) const MAX_REQUEST: usize = 8 * 1024;
pub(super) const MAX_REPLY: usize = 1024 * 1024;
pub(super) const DEADLINE: Duration = Duration::from_secs(10);

pub(super) async fn read<T: DeserializeOwned>(
    recv: &mut RecvStream,
    maximum: usize,
) -> Result<T, ShareError> {
    let mut length = [0; 4];
    recv.read_exact(&mut length)
        .await
        .map_err(|_| ShareError::Protocol)?;
    let length = u32::from_be_bytes(length) as usize;
    if length > maximum {
        return Err(ShareError::TooLarge);
    }
    let mut bytes = vec![0; length];
    recv.read_exact(&mut bytes)
        .await
        .map_err(|_| ShareError::Protocol)?;
    // FIN is part of the frame. Extra data is rejected without unbounded reads.
    let mut extra = [0];
    if recv
        .read(&mut extra)
        .await
        .map_err(|_| ShareError::Protocol)?
        .is_some()
    {
        return Err(ShareError::Protocol);
    }
    serde_json::from_slice(&bytes).map_err(|_| ShareError::Protocol)
}

pub(super) async fn write<T: Serialize>(
    send: &mut SendStream,
    message: &T,
    maximum: usize,
) -> Result<(), ShareError> {
    let mut writer = BoundedJson {
        bytes: Vec::new(),
        maximum,
        overflow: false,
    };
    if serde_json::to_writer(&mut writer, message).is_err() {
        return Err(if writer.overflow {
            ShareError::TooLarge
        } else {
            ShareError::Protocol
        });
    }
    let bytes = writer.bytes;
    send.write_all(&(bytes.len() as u32).to_be_bytes())
        .await
        .map_err(|_| ShareError::Transport)?;
    send.write_all(&bytes)
        .await
        .map_err(|_| ShareError::Transport)?;
    send.finish().map_err(|_| ShareError::Transport)
}

struct BoundedJson {
    bytes: Vec<u8>,
    maximum: usize,
    overflow: bool,
}

impl std::io::Write for BoundedJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.maximum - self.bytes.len() {
            self.overflow = true;
            return Err(std::io::Error::other("share frame too large"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
