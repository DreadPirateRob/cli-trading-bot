//! PID file management for trading bot process tracking.
//!
//! Provides functionality to create, read, and manage PID files to ensure
//! only one instance of the trading bot runs at a time.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;

/// Errors that can occur during PID file operations.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Another instance of the bot is already running.
    #[error("Trading bot is already running (PID: {pid})")]
    AlreadyRunning { pid: u32 },

    /// No running process found.
    #[error("Trading bot is not running: {0}")]
    NotRunning(String),

    /// PID file contains invalid data.
    #[error("Invalid PID file format")]
    InvalidPidFile,

    /// I/O error during file operations.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Manages a PID file for process tracking.
///
/// On creation, writes the current process ID to a file. On drop,
/// removes the file. Used to prevent multiple instances and enable
/// the stop command to signal the running process.
pub struct PidFile {
    path: PathBuf,
}

impl PidFile {
    /// Create a new PID file manager for the given path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Create the PID file with the current process ID.
    ///
    /// # Errors
    ///
    /// Returns `Error::AlreadyRunning` if another instance is running.
    /// Returns `Error::Io` if file operations fail.
    pub fn create(&self) -> Result<(), Error> {
        // Check if PID file already exists
        if self.path.exists() {
            match self.read_pid() {
                Ok(pid) => {
                    if process_exists(pid) {
                        return Err(Error::AlreadyRunning { pid });
                    }
                    // Process not running, remove stale PID file
                    tracing::warn!(pid = pid, "Removing stale PID file");
                    fs::remove_file(&self.path)?;
                }
                Err(Error::InvalidPidFile) => {
                    // Invalid PID file, remove it
                    tracing::warn!("Removing invalid PID file");
                    fs::remove_file(&self.path)?;
                }
                Err(e) => return Err(e),
            }
        }

        // Create PID file with current process ID
        let pid = std::process::id();
        let mut file = File::create(&self.path)?;
        writeln!(file, "{}", pid)?;
        tracing::debug!(pid = pid, path = %self.path.display(), "Created PID file");

        Ok(())
    }

    /// Read the PID from the file.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if the file cannot be read.
    /// Returns `Error::InvalidPidFile` if the content is not a valid PID.
    pub fn read_pid(&self) -> Result<u32, Error> {
        let mut file = File::open(&self.path)?;
        let mut content = String::new();
        file.read_to_string(&mut content)?;

        content
            .trim()
            .parse::<u32>()
            .map_err(|_| Error::InvalidPidFile)
    }

    /// Remove the PID file if it exists.
    ///
    /// # Errors
    ///
    /// Returns `Error::Io` if the file cannot be removed.
    pub fn remove(&self) -> Result<(), Error> {
        if self.path.exists() {
            fs::remove_file(&self.path)?;
            tracing::debug!(path = %self.path.display(), "Removed PID file");
        }
        Ok(())
    }

    /// Get the path to the PID file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for PidFile {
    fn drop(&mut self) {
        // Best-effort cleanup on drop, ignore errors
        let _ = self.remove();
    }
}

/// Check if a process with the given PID exists.
///
/// Uses signal 0 to check process existence without sending an actual signal.
pub fn process_exists(pid: u32) -> bool {
    // Signal 0 is used to check if process exists
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

/// Send a termination signal to a process.
///
/// # Arguments
///
/// * `pid` - The process ID to signal
/// * `signal` - The signal to send (typically SIGTERM or SIGKILL)
///
/// # Errors
///
/// Returns an error if the signal cannot be sent.
pub fn send_signal(pid: u32, signal: Signal) -> std::io::Result<()> {
    kill(Pid::from_raw(pid as i32), signal).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_create_and_read_pid() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.pid");

        let pid_file = PidFile::new(&path);
        pid_file.create().unwrap();

        let read_pid = pid_file.read_pid().unwrap();
        assert_eq!(read_pid, std::process::id());
    }

    #[test]
    fn test_remove_pid_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.pid");

        let pid_file = PidFile::new(&path);
        pid_file.create().unwrap();
        assert!(path.exists());

        pid_file.remove().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_stale_pid_file_cleanup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.pid");

        // Create a PID file with a non-existent PID
        let mut file = File::create(&path).unwrap();
        writeln!(file, "999999999").unwrap();
        drop(file);

        // Should succeed and clean up stale file
        let pid_file = PidFile::new(&path);
        pid_file.create().unwrap();

        let read_pid = pid_file.read_pid().unwrap();
        assert_eq!(read_pid, std::process::id());
    }

    #[test]
    fn test_invalid_pid_file_cleanup() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.pid");

        // Create an invalid PID file
        let mut file = File::create(&path).unwrap();
        writeln!(file, "not-a-number").unwrap();
        drop(file);

        // Should succeed and clean up invalid file
        let pid_file = PidFile::new(&path);
        pid_file.create().unwrap();

        let read_pid = pid_file.read_pid().unwrap();
        assert_eq!(read_pid, std::process::id());
    }

    #[test]
    fn test_drop_removes_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.pid");

        {
            let pid_file = PidFile::new(&path);
            pid_file.create().unwrap();
            assert!(path.exists());
        }

        // File should be removed after drop
        assert!(!path.exists());
    }

    #[test]
    fn test_process_exists_current() {
        // Current process should exist
        assert!(process_exists(std::process::id()));
    }

    #[test]
    fn test_process_not_exists() {
        // Very high PID should not exist
        assert!(!process_exists(999999999));
    }
}
