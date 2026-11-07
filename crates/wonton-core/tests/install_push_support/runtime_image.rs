use std::fs;
use std::path::Path;
use std::process::Command;

use super::{BASE, CORE, docker};

/// 把本次编译的核心及其动态库装成镜像，核心自行起的局域网探针也运行同一个程序。
pub(super) struct Image(pub(super) String);

impl Image {
    pub(super) fn build(name: String) -> Self {
        let context = tempfile::tempdir().unwrap();
        fs::copy(CORE, context.path().join("core")).unwrap();
        let libs = context.path().join("libs");
        fs::create_dir(&libs).unwrap();
        let dependencies = Command::new("ldd").arg(CORE).output().unwrap();
        assert!(dependencies.status.success());
        for line in String::from_utf8(dependencies.stdout).unwrap().lines() {
            if let Some(path) = line.split_whitespace().find(|word| word.starts_with('/')) {
                let filename = Path::new(path).file_name().unwrap();
                fs::copy(path, libs.join(filename)).unwrap();
            }
        }
        fs::copy(
            "/etc/ssl/certs/ca-certificates.crt",
            context.path().join("ca.crt"),
        )
        .unwrap();
        fs::write(
            context.path().join("Dockerfile"),
            format!(
                "FROM {BASE}\nCOPY core /usr/local/bin/wonton-core\n\
             COPY libs /usr/lib/{}-linux-gnu/\n\
             COPY ca.crt /etc/ssl/certs/ca-certificates.crt\n\
             ENTRYPOINT [\"wonton-core\"]\nCMD [\"serve\"]\n",
                std::env::consts::ARCH,
            ),
        )
        .unwrap();
        let image = Self(name);
        docker(&[
            "build",
            "--quiet",
            "--tag",
            &image.0,
            context.path().to_str().unwrap(),
        ]);
        image
    }
}

impl Drop for Image {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["image", "rm", &self.0])
            .output();
    }
}
