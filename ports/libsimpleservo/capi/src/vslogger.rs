use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use log::{self, Level, Metadata, Record};

extern {
    fn OutputDebugStringW(s: *const u16);
}

pub struct VSLogger;

impl log::Log for VSLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let log = format!("RUST: {} - {}\r\n", record.level(), record.args());
            let log: Vec<u16> = OsStr::new(&log).encode_wide().collect();
            // unsafe { OutputDebugStringW(log.as_ptr()); };
        }
    }

    fn flush(&self) {}
}
