use crate::robocopy::robocopy_header::RobocopyHeader;
use crate::robocopy::robocopy_log_entry::RobocopyLogEntry;
use chrono::Local;
use chrono::TimeZone;
use eyre::WrapErr;
use std::path::PathBuf;
use uom::si::information::byte;
use uom::si::information::mebibyte;
use uom::si::u64::Information;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum InternalState {
    ReadingHeader,
    ReadingEntries,
}

#[derive(Debug)]
pub struct RobocopyLogParser {
    buf: String,
    state: InternalState,
    // header building helpers
    header_dash_count: u8,
    header_scan_pos: usize,
    // For tracking an in‑progress New File entry
    pending_new_file: Option<PendingNewFile>,
}

#[derive(Debug)]
struct PendingNewFile {
    size: Information,
    path: PathBuf,
    percentages: Vec<u8>,
    saw_100: bool,
}

impl RobocopyLogParser {
    pub fn new() -> Self {
        Self {
            buf: String::new(),
            state: InternalState::ReadingHeader,
            header_dash_count: 0,
            header_scan_pos: 0,
            pending_new_file: None,
        }
    }

    /// Accept a newly tailed chunk from the log file.
    pub fn accept(&mut self, chunk: &str) {
        self.buf.push_str(chunk);
    }

    /// Attempt to advance the parser. Returns NeedMoreData if no complete item yet.
    pub fn advance(&mut self) -> eyre::Result<RobocopyParseAdvance> {
        match self.state {
            InternalState::ReadingHeader => self.try_parse_header(),
            InternalState::ReadingEntries => self.try_parse_entry(),
        }
    }

    fn try_parse_header(&mut self) -> eyre::Result<RobocopyParseAdvance> {
        // Stream over new data only (from header_scan_pos)
        let mut scan_pos = self.header_scan_pos;
        while let Some(rel_nl) = self.buf[scan_pos..].find('\n') {
            let line_end = scan_pos + rel_nl + 1; // include \n
            let line = &self.buf[scan_pos..line_end];
            let trimmed = line.trim_end_matches(['\r', '\n']).trim();
            if !trimmed.is_empty() && trimmed.chars().all(|c| c == '-') {
                self.header_dash_count += 1;
                if self.header_dash_count == 3 {
                    // header complete including this dashed line
                    let header_block = self.buf[..line_end].to_string();
                    // consume all header bytes
                    self.buf.drain(..line_end);
                    self.header_scan_pos = 0; // reset (buffer now shorter)
                    let header: RobocopyHeader = header_block
                        .parse()
                        .wrap_err("Failed to parse robocopy header")?;
                    self.state = InternalState::ReadingEntries;
                    return Ok(RobocopyParseAdvance::Header(header));
                }
            }
            scan_pos = line_end;
        }
        // Update scan position for next attempt
        self.header_scan_pos = scan_pos;
        Ok(RobocopyParseAdvance::NeedMoreData)
    }

    fn try_parse_entry(&mut self) -> eyre::Result<RobocopyParseAdvance> {
        loop {
            // If we are accumulating a New File entry percentages, handle that first
            if let Some(pending) = &mut self.pending_new_file {
                // Attempt to read next complete line
                if let Some(pos) = self.buf.find('\n') {
                    let line = self.buf[..pos + 1].to_string();
                    self.buf.drain(..pos + 1);
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        // skip blank line
                        continue;
                    }
                    if let Some(pct) = parse_percentage_line(trimmed) {
                        pending.percentages.push(pct);
                        if pct == 100 {
                            pending.saw_100 = true;
                        }
                        // Continue gathering until we saw 100%
                        if pending.saw_100 {
                            let finished = self.pending_new_file.take().unwrap();
                            return Ok(RobocopyParseAdvance::LogEntry(RobocopyLogEntry::NewFile {
                                size: finished.size,
                                path: finished.path,
                                percentages: finished.percentages,
                            }));
                        } else {
                            continue; // need more percentages
                        }
                    } else if is_new_file_line(trimmed) {
                        // Edge case: New file started before 100% of previous; finalize previous anyway.
                        let finished = self.pending_new_file.take().unwrap();
                        // put the line back for reprocessing as a new file start
                        self.buf.insert_str(0, &format!("{}\n", trimmed));
                        return Ok(RobocopyParseAdvance::LogEntry(RobocopyLogEntry::NewFile {
                            size: finished.size,
                            path: finished.path,
                            percentages: finished.percentages,
                        }));
                    } else {
                        // Unexpected line: finalize previous (without 100%) and reprocess this line
                        let finished = self.pending_new_file.take().unwrap();
                        self.buf.insert_str(0, &format!("{}\n", trimmed));
                        return Ok(RobocopyParseAdvance::LogEntry(RobocopyLogEntry::NewFile {
                            size: finished.size,
                            path: finished.path,
                            percentages: finished.percentages,
                        }));
                    }
                } else {
                    return Ok(RobocopyParseAdvance::NeedMoreData);
                }
            } else {
                // Not in a pending New File entry
                // Need at least one complete line
                let Some(line_end) = self.buf.find('\n') else {
                    return Ok(RobocopyParseAdvance::NeedMoreData);
                };
                let line_with_nl = self.buf[..line_end + 1].to_string();
                self.buf.drain(..line_end + 1);
                let line = line_with_nl.trim_end_matches(['\r', '\n']);
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Access denied error multi-line: date line followed by Access is denied.
                if let Some(access_err) = parse_access_denied_first_line(trimmed)? {
                    // Need the following line
                    let next_line_end = match self.buf.find('\n') {
                        Some(p) => p,
                        None => {
                            // Put the first line back and wait for more data
                            self.buf.insert_str(0, &format!("{}\n", line));
                            return Ok(RobocopyParseAdvance::NeedMoreData);
                        }
                    };
                    let next_line = self.buf[..next_line_end + 1].to_string();
                    self.buf.drain(..next_line_end + 1);
                    let next_trim = next_line.trim();
                    if next_trim.eq_ignore_ascii_case("Access is denied.") {
                        return Ok(RobocopyParseAdvance::LogEntry(access_err));
                    } else {
                        eyre::bail!(
                            "Expected 'Access is denied.' after access error line, got '{}'.",
                            next_trim
                        );
                    }
                }

                if is_new_file_line(trimmed) {
                    if let Some((size, path)) = parse_new_file_line(trimmed) {
                        self.pending_new_file = Some(PendingNewFile {
                            size,
                            path,
                            percentages: Vec::new(),
                            saw_100: false,
                        });
                        // loop to collect percentages
                        continue;
                    } else {
                        eyre::bail!("Failed to parse New File line: '{}'", trimmed);
                    }
                }
                // Lines consisting solely of percentages can appear if they race with reading; ignore until associated entry.
                if parse_percentage_line(trimmed).is_some() {
                    // Without a pending new file, we can't attach these percentages; ignore.
                    continue;
                }
                // Unknown line; just need more data (or ignore). We choose to ignore silently for now.
                continue;
            }
        }
    }
}

#[derive(Debug)]
pub enum RobocopyParseAdvance {
    NeedMoreData,
    Header(RobocopyHeader),
    LogEntry(RobocopyLogEntry),
}

fn parse_percentage_line(s: &str) -> Option<u8> {
    let t = s.trim();
    if let Some(stripped) = t.strip_suffix('%') {
        let num = stripped.trim();
        if num.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(v) = num.parse::<u16>() {
                if v <= 100 {
                    return Some(v as u8);
                }
            }
        }
    }
    None
}

fn parse_access_denied_first_line(line: &str) -> eyre::Result<Option<RobocopyLogEntry>> {
    // Format: YYYY/MM/DD HH:MM:SS ERROR <code> (<hex>) Copying Directory <PATH>\
    // We'll be lenient.
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 7 {
        return Ok(None);
    }
    if parts[0].len() == 10
        && parts[0].chars().nth(4) == Some('/')
        && parts[0].chars().nth(7) == Some('/')
    {
        // date-like
        if parts[1].len() == 8
            && parts[1].chars().nth(2) == Some(':')
            && parts[1].chars().nth(5) == Some(':')
        {
            if parts[2].eq_ignore_ascii_case("ERROR") && parts[4].starts_with("(0x") {
                // find "Copying" and "Directory"
                if parts[5].eq_ignore_ascii_case("Copying")
                    && parts[6].eq_ignore_ascii_case("Directory")
                {
                    // path remainder (joined with spaces) after 'Directory'
                    // Slice after the "Directory" token occurrence.
                    if let Some(dir_pos) = line.find("Directory") {
                        let after = &line[dir_pos + "Directory".len()..].trim();
                        let path_str = after; // includes trailing backslash per format
                        // reconstruct datetime
                        let date = parts[0];
                        let time = parts[1];
                        // date format yyyy/mm/dd time HH:MM:SS
                        let year: i32 = date[0..4].parse()?;
                        let month: u32 = date[5..7].parse()?;
                        let day: u32 = date[8..10].parse()?;
                        let hour: u32 = time[0..2].parse()?;
                        let minute: u32 = time[3..5].parse()?;
                        let second: u32 = time[6..8].parse()?;
                        let when = Local
                            .with_ymd_and_hms(year, month, day, hour, minute, second)
                            .single()
                            .ok_or_else(|| {
                                eyre::eyre!("Invalid timestamp in access denied line")
                            })?;
                        return Ok(Some(RobocopyLogEntry::AccessDeniedError {
                            when,
                            path: PathBuf::from(path_str),
                        }));
                    }
                }
            }
        }
    }
    Ok(None)
}

fn is_new_file_line(line: &str) -> bool {
    line.contains("New File")
}

fn parse_new_file_line(line: &str) -> Option<(Information, PathBuf)> {
    // Strategy: split by tabs; filter out empty trimmed segments.
    let segs: Vec<&str> = line
        .split('\t')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if segs.is_empty() {
        return None;
    }
    if !segs[0].eq_ignore_ascii_case("New File") {
        return None;
    }
    if segs.len() < 3 {
        return None;
    }
    let path_str = segs.last().unwrap();
    // size may be like "50.0 m" or "204576"
    let size_seg = segs[segs.len() - 2];
    let bytes = parse_size_to_bytes(size_seg)?;
    let info = Information::new::<byte>(bytes as u64);
    Some((info, PathBuf::from(path_str)))
}

fn parse_size_to_bytes(s: &str) -> Option<f64> {
    let t = s.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    let mut chars = t.chars().rev();
    let unit_char = chars.next().unwrap();
    let (num_str, unit) = if unit_char.is_ascii_alphabetic() {
        (&t[..t.len() - 1], Some(unit_char))
    } else {
        (&t[..], None)
    };
    let number: f64 = num_str.trim().parse().ok()?;
    let factor = match unit {
        None => 1.0,
        Some('k') => 1024.0,
        Some('m') => 1024.0 * 1024.0,
        Some('g') => 1024.0 * 1024.0 * 1024.0,
        Some('t') => 1024.0 * 1024.0 * 1024.0 * 1024.0,
        Some(_) => return None,
    };
    Some(number * factor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::robocopy::robocopy_log_entry::RobocopyLogEntry;

    #[test]
    fn parse_header_and_first_entries_streaming() -> eyre::Result<()> {
        let sample = include_str!("sample.txt");
        let mut parser = RobocopyLogParser::new();
        let mut header: Option<RobocopyHeader> = None;
        let mut entries: Vec<RobocopyLogEntry> = Vec::new();
        for chunk in sample.as_bytes().chunks(37) {
            // arbitrary chunk size
            parser.accept(std::str::from_utf8(chunk).unwrap());
            loop {
                let resp = parser.advance()?;
                println!("{resp:?}");
                match resp {
                    RobocopyParseAdvance::NeedMoreData => break,
                    RobocopyParseAdvance::Header(h) => {
                        assert!(header.is_none(), "Header emitted twice");
                        assert_eq!(h.source, PathBuf::from("J:/"));
                        header = Some(h);
                    }
                    RobocopyParseAdvance::LogEntry(entry) => entries.push(entry),
                }
            }
        }
        // Build expected entries (up to first NewFile completion present in sample excerpt)
        use chrono::TimeZone;
        let when = Local.with_ymd_and_hms(2025, 8, 27, 22, 19, 37).unwrap();
        let expected = vec![
            RobocopyLogEntry::AccessDeniedError {
                when,
                path: PathBuf::from(r"J:\$RECYCLE.BIN\"),
            },
            RobocopyLogEntry::AccessDeniedError {
                when,
                path: PathBuf::from(r"J:\System Volume Information\"),
            },
            RobocopyLogEntry::NewFile {
                size: Information::new::<mebibyte>(50),
                path: PathBuf::from(r"J:\nas-ds418j_1.hbk\Pool\0\17\0.bucket"),
                percentages: vec![5, 17, 23, 29, 35, 41, 53, 59, 65, 67, 75, 83, 89, 95, 100],
            },
            RobocopyLogEntry::NewFile {
                size: Information::new::<byte>(204576),
                path: PathBuf::from(r"J:\nas-ds418j_1.hbk\Pool\0\17\0.index"),
                percentages: vec![100],
            },
            RobocopyLogEntry::NewFile {
                size: Information::new::<byte>(0),
                path: PathBuf::from(r"J:\nas-ds418j_1.hbk\Pool\0\17\0.lock"),
                percentages: vec![100],
            },
            RobocopyLogEntry::NewFile {
                size: Information::new::<mebibyte>(50),
                path: PathBuf::from(r"J:\nas-ds418j_1.hbk\Pool\0\17\1.bucket"),
                percentages: vec![91, 97, 100],
            },
            RobocopyLogEntry::NewFile {
                size: Information::new::<byte>(204224),
                path: PathBuf::from(r"J:\nas-ds418j_1.hbk\Pool\0\17\1.index"),
                percentages: vec![100],
            },
            RobocopyLogEntry::NewFile {
                size: Information::new::<byte>(0),
                path: PathBuf::from(r"J:\nas-ds418j_1.hbk\Pool\0\17\1.lock"),
                percentages: vec![100],
            },
        ];
        assert_eq!(entries, expected, "Parsed entries mismatch");
        Ok(())
    }
}
