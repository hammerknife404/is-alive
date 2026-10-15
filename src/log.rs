//! Session log: every finished run, timestamped, kept in memory with a cap.

use std::collections::VecDeque;

/// Oldest entries are dropped past either limit.
const MAX_ENTRIES: usize = 500;
const MAX_BYTES: usize = 8 * 1024 * 1024;

pub fn now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

/// Text logged for a finished run: its output, else its error, else a placeholder.
pub fn body(output: Option<String>, err: &Option<String>) -> String {
    output
        .or_else(|| err.as_ref().map(|e| format!("error: {e}")))
        .unwrap_or_else(|| "(no results)".into())
}

pub struct Entry {
    pub tool: &'static str,
    pub started: String,
    pub ended: String,
    pub text: String,
}

impl Entry {
    pub fn header(&self) -> String {
        format!(
            "{}  started {}, ended {}",
            self.tool, self.started, self.ended
        )
    }

    pub fn render(&self) -> String {
        format!("=== {} ===\n{}\n", self.header(), self.text.trim_end())
    }
}

#[derive(Default)]
pub struct Log {
    pub entries: VecDeque<Entry>,
    bytes: usize,
    pub dropped: usize,
}

impl Log {
    pub fn push(&mut self, e: Entry) {
        self.bytes += e.text.len();
        self.entries.push_back(e);
        while self.entries.len() > MAX_ENTRIES || (self.bytes > MAX_BYTES && self.entries.len() > 1)
        {
            if let Some(old) = self.entries.pop_front() {
                self.bytes -= old.text.len();
                self.dropped += 1;
            }
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn render(&self) -> String {
        let mut s = String::new();
        if self.dropped > 0 {
            s += &format!("({} older entries dropped)\n\n", self.dropped);
        }
        for e in &self.entries {
            s += &e.render();
            s += "\n";
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: usize) -> Entry {
        Entry {
            tool: "Ping",
            started: "a".into(),
            ended: "b".into(),
            text: "x".repeat(n),
        }
    }

    #[test]
    fn caps_entries_and_bytes() {
        let mut log = Log::default();
        for _ in 0..MAX_ENTRIES + 5 {
            log.push(entry(1));
        }
        assert_eq!(log.entries.len(), MAX_ENTRIES);
        assert_eq!(log.dropped, 5);

        let mut log = Log::default();
        log.push(entry(MAX_BYTES / 2));
        log.push(entry(MAX_BYTES / 2));
        log.push(entry(10));
        assert_eq!(log.entries.len(), 2);
        // A single oversized entry is still kept.
        log.push(entry(MAX_BYTES * 2));
        assert_eq!(log.entries.len(), 1);
    }
}
