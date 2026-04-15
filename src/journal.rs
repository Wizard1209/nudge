use std::path::Path;

pub struct JournalEntry {
    pub timestamp: String,
    pub doing: String,
    pub bullshit: String,
    pub next_minutes: u32,
}

/// Append a journal entry as a CSV line to the given file.
/// Creates the file with a header if it doesn't exist.
pub fn write_entry(path: &Path, entry: &JournalEntry) {
    use std::fs::OpenOptions;
    use std::io::Write;

    let needs_header = !path.exists();

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("failed to open journal file");

    if needs_header {
        writeln!(file, "timestamp,doing,bullshit,next_minutes").expect("failed to write header");
    }

    writeln!(
        file,
        "{},{},{},{}",
        entry.timestamp, entry.doing, entry.bullshit, entry.next_minutes
    )
    .expect("failed to write entry");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn writes_csv_with_header_and_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.csv");

        let entry = JournalEntry {
            timestamp: "2026-04-14T12:00:00".to_string(),
            doing: "writing tests".to_string(),
            bullshit: "no".to_string(),
            next_minutes: 10,
        };

        write_entry(&path, &entry);

        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(lines.len(), 2, "should have header + 1 entry");
        assert_eq!(lines[0], "timestamp,doing,bullshit,next_minutes");
        assert_eq!(lines[1], "2026-04-14T12:00:00,writing tests,no,10");
    }

    #[test]
    fn appends_to_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.csv");

        let entry1 = JournalEntry {
            timestamp: "2026-04-14T12:00:00".to_string(),
            doing: "first".to_string(),
            bullshit: "no".to_string(),
            next_minutes: 10,
        };
        let entry2 = JournalEntry {
            timestamp: "2026-04-14T12:10:00".to_string(),
            doing: "second".to_string(),
            bullshit: "maybe".to_string(),
            next_minutes: 5,
        };

        write_entry(&path, &entry1);
        write_entry(&path, &entry2);

        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(lines.len(), 3, "should have header + 2 entries");
        assert_eq!(lines[2], "2026-04-14T12:10:00,second,maybe,5");
    }
}
