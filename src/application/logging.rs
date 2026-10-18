//! Bounded file logging without changing LOG_FILE or the stdout journal stream.
//! One mutex protects rotation and writes so records never go to retired files.
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};
const MAX_BYTES: u64 = 10 * 1024 * 1024;
const BACKUPS: usize = 3;

struct LogFile {
    path: PathBuf,
    file: File,
    bytes: u64,
}

impl LogFile {
    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        for index in (1..BACKUPS).rev() {
            let source = backup(&self.path, index);
            if source.exists() {
                std::fs::rename(source, backup(&self.path, index + 1))?;
            }
        }
        std::fs::rename(&self.path, backup(&self.path, 1))?;
        self.file = open(&self.path)?;
        self.bytes = 0;
        Ok(())
    }
}
fn backup(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}
fn open(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

pub struct TeeMakeWriter {
    file: Arc<Mutex<LogFile>>,
}
impl TeeMakeWriter {
    pub fn new(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let file = open(path)?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            file: Arc::new(Mutex::new(LogFile {
                path: path.into(),
                file,
                bytes,
            })),
        })
    }
}

pub struct TeeWriter<'a> {
    file: MutexGuard<'a, LogFile>,
}
impl Write for TeeWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = io::stdout().write_all(buf);
        if self.file.bytes > 0 && self.file.bytes.saturating_add(buf.len() as u64) > MAX_BYTES {
            self.file.rotate()?;
        }
        self.file.file.write_all(buf)?;
        self.file.bytes += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.file.flush()
    }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for TeeMakeWriter {
    type Writer = TeeWriter<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter {
            file: self
                .file
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing_subscriber::fmt::MakeWriter;
    #[test]
    fn rotation_keeps_a_bounded_history_at_the_configured_path() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shiny.log");
        let writer = TeeMakeWriter::new(&path).unwrap();
        for index in 0..6 {
            // Simulate a file at its threshold without printing megabytes.
            writer.file.lock().unwrap().bytes = MAX_BYTES;
            writeln!(writer.make_writer(), "record {index}").unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "record 5\n");
        assert!(backup(&path, BACKUPS).is_file());
        assert!(!backup(&path, BACKUPS + 1).exists());
    }
}
