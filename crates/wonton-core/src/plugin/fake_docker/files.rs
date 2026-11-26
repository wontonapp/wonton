//! 假 Docker 的容器归档（`/containers/{name}/archive`）用的 tar 序列化和测试入口。

use super::{FakeDocker, FakeFile};

impl FakeDocker {
    /// 让容器里出现（`Some`）或去掉（`None`）一个路径，比如插件运行中写下的文件。
    pub fn set_file(&self, container: &str, path: &str, file: Option<FakeFile>) {
        let key = (container.to_owned(), path.to_owned());
        let mut docker = self.lock();
        match file {
            Some(file) => docker.files.insert(key, file),
            None => docker.files.remove(&key),
        };
    }

    /// 最近一次写进这个容器的 tar 归档。
    pub fn archive(&self, container: &str) -> Option<Vec<u8>> {
        self.lock().archives.get(container).cloned()
    }
}

/// 和 Docker 一样：归档里只有这一项，名字是路径的最后一级。
pub(super) fn archive_of(path: &str, file: &FakeFile) -> Vec<u8> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let mut builder = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    let data: &[u8] = match file {
        FakeFile::Regular(data) => {
            header.set_entry_type(tar::EntryType::Regular);
            data
        }
        FakeFile::Directory => {
            header.set_entry_type(tar::EntryType::Directory);
            &[]
        }
    };
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    builder.append_data(&mut header, name, data).unwrap();
    builder.into_inner().unwrap()
}
