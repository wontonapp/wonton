//! 安装目录的 `.env`：一键升级只改 `WONTON_CORE_IMAGE` 这一行，别的行原样留着。

use std::fs;
use std::path::Path;

use anyhow::Context;

use super::state::write_atomic;

/// compose 文件和 `init` 离不开的键。缺了说明文件坏了，不写回去。
const REQUIRED: [&str; 6] = [
    "WONTON_CORE_IMAGE",
    "WONTON_INSTALL_DIR",
    "WONTON_DATA_DIR",
    "WONTON_PORT",
    "WONTON_TUNNEL_PORT",
    "WONTON_BASE_DOMAIN",
];

#[derive(Debug)]
pub struct EnvFile {
    lines: Vec<String>,
}

impl EnvFile {
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        // 文件操作的上下文是原始值（和系统报错一样），不进 Fluent；句子在上面的层写。
        let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Ok(Self {
            lines: text.lines().map(str::to_owned).collect(),
        })
    }

    /// [`REQUIRED`] 里第一个缺的键（没写或为空）；都齐时是 `None`。缺了说明文件坏了，不能写回去。
    pub fn missing_required_key(&self) -> Option<&'static str> {
        REQUIRED
            .iter()
            .find(|key| self.get(key).is_none_or(str::is_empty))
            .copied()
    }

    /// 同一个键写了几次时最后一次算数，和 compose 一样。
    pub fn get(&self, key: &str) -> Option<&str> {
        self.lines
            .iter()
            .rev()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
    }

    /// 换掉核心镜像，原子写回。
    pub fn write_image(mut self, path: &Path, image: &str) -> anyhow::Result<()> {
        let prefix = "WONTON_CORE_IMAGE=";
        self.lines.retain(|line| !line.starts_with(prefix));
        self.lines.push(format!("{prefix}{image}"));
        let mut text = self.lines.join("\n");
        text.push('\n');
        write_atomic(path, text.as_bytes(), 0o600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# 注释\nWONTON_CORE_SOURCE=r:stable\nWONTON_CORE_IMAGE=r@old\nWONTON_INSTALL_DIR=/opt/wonton\n\
                        WONTON_DATA_DIR=/var/lib/wonton\nWONTON_PORT=8480\nWONTON_TUNNEL_PORT=8482\n\
                        WONTON_BASE_DOMAIN=wonton.localhost\nTZ=\n";

    fn temp() -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("wonton-env-{}", crate::secret::random_token()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn only_the_image_line_changes() {
        let dir = temp();
        let path = dir.join(".env");
        fs::write(&path, FILE).unwrap();
        EnvFile::read(&path)
            .unwrap()
            .write_image(&path, "r@new")
            .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("WONTON_CORE_IMAGE=r@new\n"));
        assert!(!text.contains("r@old"));
        assert!(text.contains("# 注释\nWONTON_CORE_SOURCE=r:stable\n"));
        assert!(text.contains("TZ=\n"));
        assert_eq!(
            EnvFile::read(&path).unwrap().get("WONTON_CORE_IMAGE"),
            Some("r@new")
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// 缺必需的键就不写回：写回去也起不来，还丢了能看出问题的原文件。句子由调用方按语言写。
    #[test]
    fn a_file_missing_required_keys_is_refused() {
        let dir = temp();
        let path = dir.join(".env");
        fs::write(&path, "WONTON_CORE_IMAGE=r@old\n").unwrap();
        assert_eq!(
            EnvFile::read(&path).unwrap().missing_required_key(),
            Some("WONTON_INSTALL_DIR")
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
