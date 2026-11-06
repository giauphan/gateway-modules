//! Kernel-level inter-process lock using atomic file creation (`O_CREAT | O_EXCL`).

use std::fs;
use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub struct ProcessFileLock {
    path: PathBuf,
}

impl ProcessFileLock {
    pub fn acquire(path: &Path) -> Result<Self, String> {
        let start = Instant::now();
        let timeout = Duration::from_secs(5);
        let stale_threshold = Duration::from_secs(30);

        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(file) => {
                    use std::io::Write;
                    let pid = std::process::id();
                    let _ = write!(&file, "{pid}");
                    return Ok(Self {
                        path: path.to_path_buf(),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if let Ok(meta) = fs::metadata(path) {
                        if let Some(elapsed) = meta.modified().ok().and_then(|m| m.elapsed().ok()) {
                            if elapsed > stale_threshold {
                                let _ = fs::remove_file(path);
                                continue;
                            }
                        }
                    }

                    if start.elapsed() > timeout {
                        return Err(format!(
                            "Timeout acquiring inter-process lock on {path:?} after {:?}",
                            start.elapsed()
                        ));
                    }
                    sleep(Duration::from_millis(10));
                }
                Err(e) => return Err(format!("Failed to open lock file {path:?}: {e}")),
            }
        }
    }
}

impl Drop for ProcessFileLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
