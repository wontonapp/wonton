//! 每种语言的 Fluent 目录按组分成多个文件（`locales/<语言>/*.ftl`），这里按文件名顺序拼成一份，
//! 由 `src/l10n.rs` 嵌进二进制。分文件是为了不相干的功能不再改到同一个文案文件；
//! 两个文件里写了同一个消息 ID，`check_catalog` 和 `Catalog::new` 会报错。

use std::fs;
use std::path::Path;

const LANGUAGES: [&str; 4] = ["en", "zh-Hans", "zh-Hant", "ja"];

fn main() {
    let out = std::env::var("OUT_DIR").expect("cargo sets OUT_DIR");
    println!("cargo:rerun-if-changed=locales");
    for language in LANGUAGES {
        let dir = Path::new("locales").join(language);
        println!("cargo:rerun-if-changed={}", dir.display());
        let mut files: Vec<_> = fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
            .map(|entry| entry.expect("read locale entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "ftl"))
            .collect();
        files.sort();
        let mut joined = String::new();
        for file in files {
            println!("cargo:rerun-if-changed={}", file.display());
            let text = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
            joined.push_str(&text);
            if !joined.ends_with('\n') {
                joined.push('\n');
            }
        }
        fs::write(Path::new(&out).join(format!("{language}.ftl")), joined)
            .expect("write joined locale");
    }
}
