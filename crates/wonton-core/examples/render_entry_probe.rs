//! 把一份入口配置和两份 crt-list 写进目录，供 `haproxy -c` 和真实转发核对。
//!
//! `cargo run -p wonton-core --example render_entry_probe -- <目录>`，再把自签证书放进
//! `<目录>/certs/site.pem`（证书和私钥同一个文件）。

use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(
        std::env::args()
            .nth(1)
            .expect("用法：render_entry_probe <目录>"),
    );
    let certs = dir.join("certs");
    std::fs::create_dir_all(&certs).unwrap();
    let model = wonton_core::entry_haproxy::Model::federation_only(
        "home.example.test",
        "home.example.test",
    );
    let rendered = wonton_core::entry_haproxy::render(&model).unwrap();
    for (path, text) in [
        (dir.join("haproxy.cfg"), rendered.config),
        (certs.join("public.crt-list"), rendered.public_crt_list),
        (certs.join("lan.crt-list"), rendered.lan_crt_list),
    ] {
        std::fs::write(&path, text).unwrap();
        println!("{}", path.display());
    }
}
