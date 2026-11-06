//! Kernel-level inter-process advisory lock using `libc::flock`.

use std::fs;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

pub struct ProcessFileLock {
    file: fs::File,
}

impl ProcessFileLock {
    pub fn acquire(path: &Path) -> Result<Self, String> {
        let start = Instant::now();
        let timeout = Duration::from_secs(5);

        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| format!("Failed to open lock file {path:?}: {e}"))?;

        let fd = file.as_raw_fd();

        loop {
            let ret = unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) };
            if ret == 0 {
                return Ok(Self { file });
            }

            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EWOULDBLOCK)
                || err.raw_os_error() == Some(libc::EAGAIN)
            {
                if start.elapsed() > timeout {
                    return Err(format!(
                        "Timeout acquiring kernel advisory lock on {path:?} after {:?}",
                        start.elapsed()
                    ));
                }
                sleep(Duration::from_millis(10));
            } else {
                return Err(format!("Kernel flock failed on {path:?}: {err}"));
            }
        }
    }
}

impl Drop for ProcessFileLock {
    fn drop(&mut self) {
        let fd = self.file.as_raw_fd();
        unsafe {
            libc::flock(fd, libc::LOCK_UN);
        }
    }
}
