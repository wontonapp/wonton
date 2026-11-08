use super::{CodecError, Request};

/// 与核心插件 ID 的 DNS 标签总长限制一致，单位为字节。
pub const MAX_PLUGIN_ID_LEN: usize = 63;
/// 请求类型、名字长度、最长名字和大端 u16 端口合计 67 字节。
pub const MAX_PORT_REQUEST_LEN: usize = 1 + 1 + MAX_PLUGIN_ID_LEN + 2;

/// `Port`/`Udp` 请求头：`kind:u8 | name_len:u8 | name:[u8; name_len] | port:u16be`。
///
/// 名字为核心的反向域名插件 ID：至少两段小写字母/数字标签，单个连字符可分隔标签内的部分，
/// 总长不超过 63 字节；端口为 1024..=65535。无版本、填充或结尾 NUL。
/// 编解码只处理头，剩余流字节属于上层；解码借用输入，不分配名字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortRequest<'a> {
    pub kind: Request,
    pub plugin: &'a str,
    pub port: u16,
}

impl<'a> PortRequest<'a> {
    pub fn new(kind: Request, plugin: &'a str, port: u16) -> Result<Self, CodecError> {
        if !matches!(kind, Request::Port | Request::Udp) {
            return Err(CodecError::InvalidRequest);
        }
        let mut labels = plugin.split('.');
        if plugin.len() > MAX_PLUGIN_ID_LEN
            || !labels.clone().all(valid_label)
            || labels.nth(1).is_none()
        {
            return Err(CodecError::InvalidPlugin);
        }
        if port < 1024 {
            return Err(CodecError::InvalidPort);
        }
        Ok(Self { kind, plugin, port })
    }

    /// 返回实际写入的长度；先完整校验，失败时不写入输出。
    pub fn encode(self, output: &mut [u8]) -> Result<usize, CodecError> {
        Self::new(self.kind, self.plugin, self.port)?;
        let len = self.plugin.len() + 4;
        if output.len() < len {
            return Err(CodecError::BufferTooSmall);
        }
        output[0] = self.kind as u8;
        output[1] = self.plugin.len() as u8;
        output[2..len - 2].copy_from_slice(self.plugin.as_bytes());
        output[len - 2..len].copy_from_slice(&self.port.to_be_bytes());
        Ok(len)
    }

    /// 返回头及消费字节数；后续数据不参与请求头解析。
    pub fn decode(input: &'a [u8]) -> Result<(Self, usize), CodecError> {
        let kind = *input.first().ok_or(CodecError::Truncated)?;
        let kind = Request::from_byte(kind).ok_or(CodecError::InvalidRequest)?;
        if !matches!(kind, Request::Port | Request::Udp) {
            return Err(CodecError::InvalidRequest);
        }
        let name_len = usize::from(*input.get(1).ok_or(CodecError::Truncated)?);
        if name_len > MAX_PLUGIN_ID_LEN {
            return Err(CodecError::InvalidPlugin);
        }
        let len = name_len + 4;
        let header = input.get(..len).ok_or(CodecError::Truncated)?;
        let plugin =
            std::str::from_utf8(&header[2..len - 2]).map_err(|_| CodecError::InvalidPlugin)?;
        let port = u16::from_be_bytes([header[len - 2], header[len - 1]]);
        Ok((Self::new(kind, plugin, port)?, len))
    }
}

fn valid_label(label: &str) -> bool {
    label.split('-').all(|part| {
        !part.is_empty()
            && part
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_preserves_payload_and_borrows_name() {
        for kind in [Request::Port, Request::Udp] {
            let request = PortRequest::new(kind, "app.example-video", 1024).unwrap();
            let mut bytes = [0; MAX_PORT_REQUEST_LEN + 1];
            let len = request.encode(&mut bytes).unwrap();
            bytes[len] = 42;
            let (decoded, used) = PortRequest::decode(&bytes[..len + 1]).unwrap();
            assert_eq!(decoded, request);
            assert_eq!(used, len);
            assert_eq!(bytes[used], 42);
            assert_eq!(decoded.plugin.as_ptr(), bytes[2..].as_ptr());
            for end in 0..len {
                assert!(PortRequest::decode(&bytes[..end]).is_err());
            }
        }
    }

    #[test]
    fn exact_name_port_and_buffer_limits() {
        let name = format!("a.{}", "b".repeat(61));
        assert_eq!(name.len(), MAX_PLUGIN_ID_LEN);
        let request = PortRequest::new(Request::Udp, &name, u16::MAX).unwrap();
        let mut output = [0; MAX_PORT_REQUEST_LEN];
        assert_eq!(request.encode(&mut output), Ok(MAX_PORT_REQUEST_LEN));
        assert_eq!(PortRequest::decode(&output).unwrap().0, request);
        assert_eq!(
            request.encode(&mut output[..MAX_PORT_REQUEST_LEN - 1]),
            Err(CodecError::BufferTooSmall)
        );
        let too_long = format!("{name}b");
        assert_eq!(too_long.len(), MAX_PLUGIN_ID_LEN + 1);
        assert_eq!(
            PortRequest::new(Request::Port, &too_long, 1024),
            Err(CodecError::InvalidPlugin)
        );
        assert_eq!(
            PortRequest::new(Request::Port, &name, 1023),
            Err(CodecError::InvalidPort)
        );
        output[1] = 64;
        assert_eq!(PortRequest::decode(&output), Err(CodecError::InvalidPlugin));
    }

    #[test]
    fn invalid_header_values_are_rejected() {
        for name in [
            "", "single", ".app", "app.", "app..id", "App.id", "app.-id", "app.id-", "app.i--d",
            "app.é",
        ] {
            assert_eq!(
                PortRequest::new(Request::Udp, name, 1024),
                Err(CodecError::InvalidPlugin)
            );
        }
        assert_eq!(
            PortRequest::new(Request::Ping, "app.id", 1024),
            Err(CodecError::InvalidRequest)
        );
        assert_eq!(
            PortRequest::decode(&[3, 3, b'a', b'.', 255, 4, 0]),
            Err(CodecError::InvalidPlugin)
        );
        assert_eq!(
            PortRequest::decode(&[4, 3, b'a', b'.', b'b', 3, 255]),
            Err(CodecError::InvalidPort)
        );
        assert_eq!(PortRequest::decode(&[99]), Err(CodecError::InvalidRequest));
    }
}
