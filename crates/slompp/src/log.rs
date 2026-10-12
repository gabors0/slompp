//! The panel's event log: what the user did and what the servers did, like
//! the message box at the bottom of XAMPP's control panel.

use slompp_core::Module;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Info,
    Ok,
    Error,
    /// Supporting lines under an error (log excerpts).
    Detail,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub time: String,
    /// `None` for the panel itself.
    pub module: Option<Module>,
    pub kind: Kind,
    pub text: String,
}

/// The newest entries; older ones fall off past `LIMIT`.
#[derive(Default)]
pub struct Log {
    entries: Vec<Entry>,
    /// Bumped on every change, so the console knows to scroll down.
    pub revision: usize,
}

const LIMIT: usize = 1000;

impl Log {
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn push(&mut self, module: Option<Module>, kind: Kind, text: impl Into<String>) {
        let text = text.into();
        let time = chrono::Local::now().format("%H:%M:%S").to_string();
        // Multi-line messages (errors with log excerpts): the first line keeps
        // its kind, the rest are details.
        for (i, line) in text.lines().filter(|l| !l.trim().is_empty()).enumerate() {
            self.entries.push(Entry {
                time: time.clone(),
                module,
                kind: if i == 0 { kind } else { Kind::Detail },
                text: line.to_string(),
            });
        }
        if self.entries.len() > LIMIT {
            let excess = self.entries.len() - LIMIT;
            self.entries.drain(..excess);
        }
        self.revision += 1;
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.revision += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_multi_line_messages_into_details() {
        let mut log = Log::default();
        log.push(
            Some(Module::Apache),
            Kind::Error,
            "Apache stopped while starting.\n\nerror.log:\nAH00072: make_sock",
        );
        let kinds: Vec<Kind> = log.entries().iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [Kind::Error, Kind::Detail, Kind::Detail]);
        assert_eq!(log.entries()[2].text, "AH00072: make_sock");
    }

    #[test]
    fn keeps_the_newest_entries() {
        let mut log = Log::default();
        for i in 0..LIMIT + 5 {
            log.push(None, Kind::Info, format!("line {i}"));
        }
        assert_eq!(log.entries().len(), LIMIT);
        assert_eq!(log.entries()[0].text, "line 5");
    }
}
