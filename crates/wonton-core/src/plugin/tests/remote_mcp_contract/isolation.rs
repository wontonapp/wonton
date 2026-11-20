use std::io::{Read, Seek, SeekFrom};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(super) fn run_here(name: &str) -> bool {
    const CHILD: &str = "WONTON_MCP_CONTRACT_CHILD";
    if std::env::var(CHILD).ok().as_deref() == Some(name) {
        return true;
    }
    // Separate processes isolate the SDK's spawned tasks and global callsite cache.
    let mut output = tempfile::tempfile().unwrap();
    let mut child = Running(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .env(CHILD, name)
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(45);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "isolated contract exceeded 45 seconds: {name}"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    output.seek(SeekFrom::Start(0)).unwrap();
    let mut transcript = String::new();
    output.read_to_string(&mut transcript).unwrap();
    assert!(
        status.success(),
        "isolated contract failed: {name}: {status}\n{transcript}"
    );
    assert!(
        transcript.contains("test result: ok. 1 passed;"),
        "isolated contract did not execute exactly one test: {name}\n{transcript}",
    );
    false
}
