//! 上传的压缩包（ADR-0031）：tar、tar.gz 和 zip，按内容认格式。只解压 compose 和它引用的文件，别的文件再大也不碰，
//! 所以直接压缩整个仓库也能装。什么都不写进文件系统，压缩包里的路径和链接碰不到宿主。

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Read};

use super::languages::Languages;
use super::package_files::{COMPOSE_FILE, MAX_FILE_BYTES, PackageFiles, normalize};
use crate::l10n::text;

/// 压缩包最多这么大（网关的请求体上限）。
#[allow(clippy::cast_possible_truncation)]
pub const MAX_ARCHIVE_BYTES: usize = wonton_protocol::MAX_PLUGIN_ARCHIVE_BYTES as usize;
/// tar.gz 解开后最多这么大：tar 要顺着读才找得到文件，只能先解开。
const MAX_UNPACKED_BYTES: u64 = 64 << 20;
/// 压缩包里最多这么多项。
const MAX_ENTRIES: usize = 1024;
/// macOS 的“压缩”放资源分支的目录，和插件包无关。
const MACOS_METADATA: &str = "__MACOSX";

const GZIP_MAGIC: &[u8] = &[0x1f, 0x8b];
const ZIP_MAGIC: &[u8] = b"PK\x03\x04";
const ZIP_EMPTY_MAGIC: &[u8] = b"PK\x05\x06";
const TAR_MAGIC: &[u8] = b"ustar";
const TAR_MAGIC_OFFSET: usize = 257;

/// 请求体是压缩包，而不是 compose 文本。
pub fn is_archive(body: &[u8]) -> bool {
    body.starts_with(GZIP_MAGIC)
        || body.starts_with(ZIP_MAGIC)
        || body.starts_with(ZIP_EMPTY_MAGIC)
        || body
            .get(TAR_MAGIC_OFFSET..TAR_MAGIC_OFFSET + TAR_MAGIC.len())
            .is_some_and(|magic| magic == TAR_MAGIC)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    File,
    Dir,
    /// 软链接、硬链接、设备等：被引用时拒绝，没被引用时和插件包无关。
    Other(String),
}

/// 打开的压缩包。tar.gz 已经解开成 tar。
pub struct Archive {
    format: Format,
}

enum Format {
    Tar(Vec<u8>),
    Zip(Vec<u8>),
}

impl Archive {
    pub fn open(body: &[u8]) -> Result<Self, String> {
        let format = if body.starts_with(GZIP_MAGIC) {
            let mut tar = Vec::new();
            flate2::read::GzDecoder::new(body)
                .take(MAX_UNPACKED_BYTES + 1)
                .read_to_end(&mut tar)
                .map_err(|e| format!("can't decompress gzip: {e}"))?;
            if tar.len() as u64 > MAX_UNPACKED_BYTES {
                return Err(format!(
                    "unpacks to more than {} MiB",
                    MAX_UNPACKED_BYTES >> 20
                ));
            }
            Format::Tar(tar)
        } else if body.starts_with(ZIP_MAGIC) || body.starts_with(ZIP_EMPTY_MAGIC) {
            Format::Zip(body.to_vec())
        } else {
            Format::Tar(body.to_vec())
        };
        Ok(Self { format })
    }

    /// 每一项的路径（规范化后）和种类，按压缩包里的顺序。路径不合法的（如 `../x`）不可能被引用，不列出。
    fn entries(&self, langs: &Languages) -> Result<Vec<(String, Kind)>, String> {
        let mut entries = Vec::new();
        let mut add = |raw: String, kind: Kind| {
            if entries.len() >= MAX_ENTRIES {
                // 这条没有对应的消息 ID，不按语言写。
                return Err(format!("the archive holds more than {MAX_ENTRIES} entries"));
            }
            if let Ok(path) = normalize(&raw) {
                entries.push((path, kind));
            }
            Ok(())
        };
        match &self.format {
            Format::Tar(tar) => {
                let mut archive = tar::Archive::new(tar.as_slice());
                let read = archive.entries().map_err(|e| {
                    text(
                        langs,
                        "plugin-install-arch-not-tar",
                        &[("error", e.to_string().into())],
                    )
                })?;
                for entry in read {
                    let entry = entry.map_err(|e| {
                        text(
                            langs,
                            "plugin-install-arch-read-failed",
                            &[("error", e.to_string().into())],
                        )
                    })?;
                    let raw = entry
                        .path()
                        .map_err(|e| {
                            text(
                                langs,
                                "plugin-install-arch-path-invalid",
                                &[("error", e.to_string().into())],
                            )
                        })?
                        .to_string_lossy()
                        .into_owned();
                    add(raw, tar_kind(entry.header().entry_type()))?;
                }
            }
            Format::Zip(zip) => {
                let mut archive = open_zip(zip, langs)?;
                for index in 0..archive.len() {
                    // 只看目录，不解压：没被引用的项压缩方式不支持、加了密都不要紧。
                    let entry = archive.by_index_raw(index).map_err(|e| {
                        text(
                            langs,
                            "plugin-install-arch-zip-read-failed",
                            &[("error", e.to_string().into())],
                        )
                    })?;
                    let kind = if entry.is_dir() {
                        Kind::Dir
                    } else if entry.is_symlink() {
                        Kind::Other(text(langs, "plugin-install-arch-symlink", &[]))
                    } else {
                        Kind::File
                    };
                    let name = entry
                        .name()
                        .map_err(|e| {
                            text(
                                langs,
                                "plugin-install-arch-zip-path-invalid",
                                &[("error", e.to_string().into())],
                            )
                        })?
                        .into_owned();
                    add(name, kind)?;
                }
            }
        }
        Ok(entries)
    }

    /// compose 所在的目录：根目录，或者唯一一个一级目录（GitHub 的 “Download ZIP” 把所有文件放在 `仓库-版本/` 下）。
    /// 空字符串是根目录，否则带结尾的 `/`。
    pub fn root(&self, langs: &Languages) -> Result<String, String> {
        let entries = self.entries(langs)?;
        if entries.iter().any(|(path, _)| path == COMPOSE_FILE) {
            return Ok(String::new());
        }
        let mut roots = entries.iter().filter_map(|(path, _)| {
            let (dir, name) = path.split_once('/')?;
            (name == COMPOSE_FILE && dir != MACOS_METADATA).then_some(dir)
        });
        match (roots.next(), roots.next()) {
            (Some(dir), None) => Ok(format!("{dir}/")),
            (Some(a), Some(b)) => Err(text(
                langs,
                "plugin-install-arch-two-roots",
                &[("a", a.into()), ("b", b.into())],
            )),
            (None, _) => Err(text(langs, "plugin-install-arch-missing-compose", &[])),
        }
    }

    /// 读 `root` 下的 `wanted`，路径相对 `root`。没有的文件不报错，由 [`super::package::PluginPackage`] 按引用它的字段报。
    pub fn read(
        &self,
        root: &str,
        wanted: &[String],
        langs: &Languages,
    ) -> Result<PackageFiles, String> {
        let wanted: BTreeMap<String, String> = wanted
            .iter()
            .map(|path| (format!("{root}{path}"), path.clone()))
            .collect();
        self.read_wanted(&wanted, MAX_FILE_BYTES, langs)
    }

    /// `prefix`（带结尾的 `/`）下的全部文件，路径相对 `prefix`：静态网页的目录（ADR-0080）。目录项不算文件；
    /// 软链接这类不是普通文件的项报错。
    pub fn read_tree(
        &self,
        prefix: &str,
        max: u64,
        langs: &Languages,
    ) -> Result<PackageFiles, String> {
        let mut wanted = BTreeMap::new();
        for (path, kind) in self.entries(langs)? {
            let Some(rel) = path.strip_prefix(prefix) else {
                continue;
            };
            match kind {
                Kind::File => {
                    wanted.insert(path.clone(), rel.to_owned());
                }
                Kind::Other(what) => {
                    return Err(text(
                        langs,
                        "plugin-install-arch-not-regular",
                        &[("rel", rel.into()), ("what", what.into())],
                    ));
                }
                Kind::Dir => {}
            }
        }
        self.read_wanted(&wanted, max, langs)
    }

    fn read_wanted(
        &self,
        wanted: &BTreeMap<String, String>,
        max: u64,
        langs: &Languages,
    ) -> Result<PackageFiles, String> {
        let mut seen = BTreeSet::new();
        for (path, kind) in self.entries(langs)? {
            let Some(rel) = wanted.get(&path) else {
                continue;
            };
            if let Kind::Other(what) = &kind {
                return Err(text(
                    langs,
                    "plugin-install-arch-not-regular",
                    &[("rel", rel.as_str().into()), ("what", what.into())],
                ));
            }
            // 同名的两项，不同的工具可能解出不同的那个：拒绝，主人核对摘要才有意义。
            if kind == Kind::File && !seen.insert(path.clone()) {
                return Err(text(
                    langs,
                    "plugin-install-arch-duplicate-file",
                    &[("rel", rel.as_str().into())],
                ));
            }
        }
        let mut files = PackageFiles::new();
        let mut keep = |path: &str, data: Vec<u8>| -> Result<(), String> {
            let rel = wanted.get(path).expect("only wanted files are read");
            if data.len() as u64 > max {
                return Err(format!("`{rel}` is over {} KiB", max >> 10));
            }
            files.insert(rel.clone(), data);
            Ok(())
        };
        match &self.format {
            Format::Tar(tar) => read_tar(tar, wanted, max, &mut keep, langs)?,
            Format::Zip(zip) => read_zip(zip, wanted, max, &mut keep, langs)?,
        }
        Ok(files)
    }
}

type Keep<'a> = &'a mut dyn FnMut(&str, Vec<u8>) -> Result<(), String>;

fn read_tar(
    tar: &[u8],
    wanted: &BTreeMap<String, String>,
    max: u64,
    keep: Keep<'_>,
    langs: &Languages,
) -> Result<(), String> {
    let mut archive = tar::Archive::new(tar);
    let read = archive.entries().map_err(|e| {
        text(
            langs,
            "plugin-install-arch-not-tar",
            &[("error", e.to_string().into())],
        )
    })?;
    for entry in read {
        let entry = entry.map_err(|e| {
            text(
                langs,
                "plugin-install-arch-read-failed",
                &[("error", e.to_string().into())],
            )
        })?;
        let raw = entry
            .path()
            .map_err(|e| {
                text(
                    langs,
                    "plugin-install-arch-path-invalid",
                    &[("error", e.to_string().into())],
                )
            })?
            .to_string_lossy()
            .into_owned();
        let Ok(path) = normalize(&raw) else { continue };
        if wanted.contains_key(&path) && tar_kind(entry.header().entry_type()) == Kind::File {
            let data = read_limited(entry, &path, max, langs)?;
            keep(&path, data)?;
        }
    }
    Ok(())
}

fn read_zip(
    zip: &[u8],
    wanted: &BTreeMap<String, String>,
    max: u64,
    keep: Keep<'_>,
    langs: &Languages,
) -> Result<(), String> {
    let mut archive = open_zip(zip, langs)?;
    for index in 0..archive.len() {
        let name = archive
            .by_index_raw(index)
            .map_err(|e| {
                text(
                    langs,
                    "plugin-install-arch-zip-read-failed",
                    &[("error", e.to_string().into())],
                )
            })?
            .name()
            .map_err(|e| {
                text(
                    langs,
                    "plugin-install-arch-zip-path-invalid",
                    &[("error", e.to_string().into())],
                )
            })?
            .into_owned();
        let Ok(path) = normalize(&name) else { continue };
        if !wanted.contains_key(&path) {
            continue;
        }
        let entry = archive.by_index(index).map_err(|e| {
            text(
                langs,
                "plugin-install-arch-extract-failed",
                &[
                    ("path", path.as_str().into()),
                    ("error", e.to_string().into()),
                ],
            )
        })?;
        if entry.is_file() {
            let data = read_limited(entry, &path, max, langs)?;
            keep(&path, data)?;
        }
    }
    Ok(())
}

fn open_zip<'a>(
    zip: &'a [u8],
    langs: &Languages,
) -> Result<zip::ZipArchive<Cursor<&'a [u8]>>, String> {
    zip::ZipArchive::new(Cursor::new(zip)).map_err(|e| {
        text(
            langs,
            "plugin-install-arch-zip-read-failed",
            &[("error", e.to_string().into())],
        )
    })
}

fn tar_kind(kind: tar::EntryType) -> Kind {
    match kind {
        tar::EntryType::Regular | tar::EntryType::Continuous => Kind::File,
        tar::EntryType::Directory => Kind::Dir,
        other => Kind::Other(format!("{other:?}")),
    }
}

fn read_limited(
    entry: impl Read,
    path: &str,
    max: u64,
    langs: &Languages,
) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    entry.take(max + 1).read_to_end(&mut data).map_err(|e| {
        text(
            langs,
            "plugin-install-arch-read-entry-failed",
            &[("path", path.into()), ("error", e.to_string().into())],
        )
    })?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::l10n::CATALOG;
    use crate::plugin::languages::Languages;

    fn langs() -> Languages {
        Languages::default()
    }

    /// 测试断言用的目录消息（默认语言是英文，期望值也从目录生成）。
    fn message(id: &str, args: &[(&str, &str)]) -> String {
        let lang = crate::l10n::lang(&langs());
        let args: Vec<(&str, wonton_l10n::Arg<'_>)> =
            args.iter().map(|(k, v)| (*k, (*v).into())).collect();
        CATALOG.format(lang, id, &args)
    }

    const COMPOSE: &[u8] = b"services: {}";

    fn tar_of(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (path, kind, data) in entries {
            let mut header = tar::Header::new_ustar();
            header.as_old_mut().name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            if *kind == tar::EntryType::Symlink {
                header.set_link_name("/etc/passwd").unwrap();
            }
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap()
    }

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        for (path, data) in entries {
            writer.start_file(*path, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn read_all(body: &[u8], wanted: &[&str]) -> Result<(String, PackageFiles), String> {
        let langs = langs();
        let archive = Archive::open(body)?;
        let root = archive.root(&langs)?;
        let wanted: Vec<String> = wanted.iter().map(ToString::to_string).collect();
        let files = archive.read(&root, &wanted, &langs)?;
        Ok((root, files))
    }

    #[test]
    fn tar_gzip_and_zip_are_recognized_by_their_content() {
        let regular = tar::EntryType::Regular;
        let tar = tar_of(&[
            ("./docker-compose.yml", regular, COMPOSE),
            ("./icon.png", regular, b"png"),
        ]);
        let zip = zip_of(&[("docker-compose.yml", COMPOSE), ("icon.png", b"png")]);
        for body in [tar.clone(), gz(&tar), zip] {
            assert!(is_archive(&body));
            let (root, files) = read_all(&body, &["docker-compose.yml", "icon.png"]).unwrap();
            assert_eq!(root, "");
            assert_eq!(files["icon.png"], b"png");
            assert_eq!(files["docker-compose.yml"], COMPOSE);
        }
        assert!(!is_archive(b"services:\n  a:\n    image: x\n"));
    }

    #[test]
    fn a_single_top_level_folder_is_the_root_and_macos_metadata_is_ignored() {
        let body = zip_of(&[
            ("repo-1.0/docker-compose.yml", COMPOSE),
            ("repo-1.0/icon.png", b"png"),
            ("__MACOSX/docker-compose.yml", b"resource fork"),
        ]);
        let (root, files) = read_all(&body, &["docker-compose.yml", "icon.png"]).unwrap();
        assert_eq!(root, "repo-1.0/");
        assert_eq!(files["docker-compose.yml"], COMPOSE);

        let two = zip_of(&[
            ("a/docker-compose.yml", COMPOSE),
            ("b/docker-compose.yml", COMPOSE),
        ]);
        assert_eq!(
            read_all(&two, &[]).unwrap_err(),
            message("plugin-install-arch-two-roots", &[("a", "a"), ("b", "b")])
        );
        assert!(read_all(&zip_of(&[("x", b"y")]), &[]).is_err());
    }

    #[test]
    fn only_referenced_files_are_read_so_a_whole_repository_can_be_uploaded() {
        let big = vec![b'x'; MAX_FILE_BYTES as usize + 1];
        let regular = tar::EntryType::Regular;
        let tar = tar_of(&[
            ("docker-compose.yml", regular, COMPOSE),
            ("screenshots/big.png", regular, &big),
            ("link", tar::EntryType::Symlink, b""),
        ]);
        let (_, files) = read_all(&tar, &["docker-compose.yml"]).unwrap();
        assert_eq!(files.len(), 1);

        let zip = zip_of(&[("docker-compose.yml", COMPOSE), ("big.bin", &big)]);
        assert_eq!(read_all(&zip, &["docker-compose.yml"]).unwrap().1.len(), 1);
    }

    #[test]
    fn referenced_files_are_still_held_to_the_package_rules() {
        let big = vec![b'x'; MAX_FILE_BYTES as usize + 1];
        let regular = tar::EntryType::Regular;
        let compose = ("docker-compose.yml", regular, COMPOSE);

        let oversized = tar_of(&[compose, ("big.bin", regular, &big)]);
        assert!(read_all(&oversized, &["big.bin"]).is_err());

        let link = tar_of(&[compose, ("icon.png", tar::EntryType::Symlink, b"")]);
        let err = read_all(&link, &["icon.png"]).unwrap_err();
        assert!(
            err.contains(&message("plugin-install-arch-symlink", &[])),
            "{err}"
        );

        let twice = tar_of(&[compose, ("a", regular, b"1"), ("a", regular, b"2")]);
        assert_eq!(
            read_all(&twice, &["a"]).unwrap_err(),
            message("plugin-install-arch-duplicate-file", &[("rel", "a")])
        );

        let escape = tar_of(&[compose, ("../a", regular, b"x")]);
        assert!(read_all(&escape, &["a"]).unwrap().1.is_empty());
    }

    #[test]
    fn archives_that_unpack_too_big_or_hold_too_many_entries_are_refused() {
        let bomb = gz(&vec![0; MAX_UNPACKED_BYTES as usize + 1]);
        assert!(bomb.len() < MAX_ARCHIVE_BYTES);
        assert!(
            Archive::open(&bomb)
                .err()
                .unwrap()
                .contains("unpacks to more than")
        );

        let entries: Vec<(String, &[u8])> = (0..=MAX_ENTRIES)
            .map(|i| (format!("f{i}"), b"".as_slice()))
            .collect();
        let entries: Vec<(&str, &[u8])> = entries.iter().map(|(p, d)| (p.as_str(), *d)).collect();
        let many = zip_of(&entries);
        assert!(read_all(&many, &[]).unwrap_err().contains("more than"));
    }
}
