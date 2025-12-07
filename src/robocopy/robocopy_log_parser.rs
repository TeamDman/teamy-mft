use crate::robocopy::robocopy_header::RobocopyHeader;
use crate::robocopy::robocopy_log_entry::RobocopyLogEntry;
use chrono::Local;
use chrono::TimeZone;
use eyre::WrapErr;
use std::path::PathBuf;
use uom::si::information::byte;
use uom::si::usize::Information;

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
}

impl Default for RobocopyLogParser {
    fn default() -> Self {
        Self::new()
    }
}

impl RobocopyLogParser {
    #[must_use]
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

    /// Attempt to advance the parser. Returns `NeedMoreData` if no complete item yet.
    ///
    /// # Errors
    ///
    /// Returns an error if parsing a header or log entry fails.
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

    #[allow(
        clippy::too_many_lines,
        reason = "state-machine parsing of streamed robocopy entries"
    )]
    fn try_parse_entry(&mut self) -> eyre::Result<RobocopyParseAdvance> {
        loop {
            if let Some(pending) = &mut self.pending_new_file {
                // try to read next segment (handle CR or LF)
                let pos = match (self.buf.find('\n'), self.buf.find('\r')) {
                    (Some(nl), Some(cr)) => Some(nl.min(cr)),
                    (Some(nl), None) => Some(nl),
                    (None, Some(cr)) => Some(cr),
                    (None, None) => None,
                };
                let Some(end) = pos else {
                    return Ok(RobocopyParseAdvance::NeedMoreData);
                };
                let seg_with_term = self.buf[..=end].to_string();
                self.buf.drain(..=end);
                for piece in seg_with_term
                    .split(['\r', '\n'])
                    .filter(|p| !p.trim().is_empty())
                {
                    let trimmed = piece.trim();
                    if let Some(pct) = parse_percentage_line(trimmed) {
                        pending.percentages.push(pct);
                        if pct == 100 {
                            // finalize and emit final state
                            let finished = self.pending_new_file.take().unwrap();
                            return Ok(RobocopyParseAdvance::LogEntry(RobocopyLogEntry::NewFile {
                                size: finished.size,
                                path: finished.path,
                                percentages: finished.percentages,
                            }));
                        }
                        // Emit incremental state (clone path)
                        return Ok(RobocopyParseAdvance::LogEntry(RobocopyLogEntry::NewFile {
                            size: pending.size,
                            path: pending.path.clone(),
                            percentages: pending.percentages.clone(),
                        }));
                    } else if is_new_file_line(trimmed) {
                        // finalize previous without adding new pct and reprocess line
                        let finished = self.pending_new_file.take().unwrap();
                        self.buf.insert_str(0, &format!("{trimmed}\n"));
                        return Ok(RobocopyParseAdvance::LogEntry(RobocopyLogEntry::NewFile {
                            size: finished.size,
                            path: finished.path,
                            percentages: finished.percentages,
                        }));
                    }
                    // ignore noise
                }
                // more data is required to make progress for pending item
                continue;
            }

            // read line/segment for new entry when no pending file
            let pos = match (self.buf.find('\n'), self.buf.find('\r')) {
                (Some(nl), Some(cr)) => Some(nl.min(cr)),
                (Some(nl), None) => Some(nl),
                (None, Some(cr)) => Some(cr),
                (None, None) => None,
            };
            let Some(end) = pos else {
                return Ok(RobocopyParseAdvance::NeedMoreData);
            };
            let seg_with_term = self.buf[..=end].to_string();
            self.buf.drain(..=end);
            let mut pieces: Vec<&str> = seg_with_term
                .split(['\r', '\n'])
                .filter(|p| !p.trim().is_empty())
                .collect();
            if pieces.is_empty() {
                continue;
            }
            let first = pieces.remove(0);
            if !pieces.is_empty() {
                let rest = pieces.join("\n");
                self.buf.insert_str(0, &format!("{rest}\n"));
            }
            let trimmed = first.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(access_err) = parse_access_denied_first_line(trimmed)? {
                // Peek if we have a following line terminator at all
                let pos2 = match (self.buf.find('\n'), self.buf.find('\r')) {
                    (Some(nl), Some(cr)) => Some(nl.min(cr)),
                    (Some(nl), None) => Some(nl),
                    (None, Some(cr)) => Some(cr),
                    (None, None) => None,
                };
                let Some(end2) = pos2 else {
                    // not enough data, push back and wait
                    self.buf.insert_str(0, &format!("{trimmed}\n"));
                    return Ok(RobocopyParseAdvance::NeedMoreData);
                };
                let second_line = self.buf[..=end2].to_string();
                self.buf.drain(..=end2);
                if second_line.trim().eq_ignore_ascii_case("Access is denied.") {
                    return Ok(RobocopyParseAdvance::LogEntry(access_err));
                } else if second_line.trim().is_empty() {
                    // blank line after; accept the error anyway
                    return Ok(RobocopyParseAdvance::LogEntry(access_err));
                }
                // Unexpected; reinsert consumed second line for future processing and still emit error.
                self.buf.insert_str(0, &second_line);
                return Ok(RobocopyParseAdvance::LogEntry(access_err));
            }
            if is_new_file_line(trimmed) {
                if let Some((size, path)) = parse_new_file_line(trimmed) {
                    self.pending_new_file = Some(PendingNewFile {
                        size,
                        path: path.clone(),
                        percentages: Vec::new(),
                    });
                    // emit initial new file with empty percentages
                    return Ok(RobocopyParseAdvance::LogEntry(RobocopyLogEntry::NewFile {
                        size,
                        path,
                        percentages: Vec::new(),
                    }));
                }
                eyre::bail!("Failed to parse New File line: '{}'", trimmed);
            }
            // percentage lines at top-level are ignored; nothing to do here
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum RobocopyParseAdvance {
    NeedMoreData,
    Header(RobocopyHeader),
    LogEntry(RobocopyLogEntry),
}

fn parse_percentage_line(s: &str) -> Option<u8> {
    let t = s.trim();
    if let Some(stripped) = t.strip_suffix('%') {
        let num = stripped.trim();
        if num.chars().all(|c| c.is_ascii_digit())
            && let Ok(v) = num.parse::<u16>()
            && v <= 100
            && let Ok(pct) = u8::try_from(v)
        {
            return Some(pct);
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
            && parts[2].eq_ignore_ascii_case("ERROR")
            && parts[4].starts_with("(0x")
        {
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
                        .ok_or_else(|| eyre::eyre!("Invalid timestamp in access denied line"))?;
                    return Ok(Some(RobocopyLogEntry::AccessDeniedError {
                        when,
                        path: PathBuf::from(path_str),
                    }));
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
        .map(str::trim)
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
    Some((bytes, PathBuf::from(path_str)))
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "robocopy output contains human-readable floats that we round to usize"
)]
fn parse_size_to_bytes(s: &str) -> Option<Information> {
    let t = s.trim().to_lowercase();
    if t.is_empty() {
        return None;
    }
    let mut chars = t.chars().rev();
    let unit_char = chars.next().unwrap();
    let (num_str, unit) = if unit_char.is_ascii_alphabetic() {
        (&t[..t.len() - 1], Some(unit_char))
    } else {
        ((&*t), None)
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
    Some(Information::new::<byte>((number * factor) as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use uom::si::information::mebibyte;

    fn push_new_file(
        expected: &mut Vec<RobocopyLogEntry>,
        size: Information,
        path: &str,
        percentages: &[u8],
    ) {
        // initial empty
        expected.push(RobocopyLogEntry::NewFile {
            size,
            path: PathBuf::from(path),
            percentages: Vec::new(),
        });
        let mut acc: Vec<u8> = Vec::new();
        for &p in percentages {
            acc.push(p);
            expected.push(RobocopyLogEntry::NewFile {
                size,
                path: PathBuf::from(path),
                percentages: acc.clone(),
            });
        }
    }

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
        let when = Local.with_ymd_and_hms(2025, 8, 27, 22, 19, 37).unwrap();
        // Build expected incremental emissions
        let mut expected: Vec<RobocopyLogEntry> = Vec::new();
        expected.push(RobocopyLogEntry::AccessDeniedError {
            when,
            path: PathBuf::from(r"J:\$RECYCLE.BIN\"),
        });
        expected.push(RobocopyLogEntry::AccessDeniedError {
            when,
            path: PathBuf::from(r"J:\System Volume Information\"),
        });
        push_new_file(
            &mut expected,
            Information::new::<mebibyte>(50),
            r"J:\nas-ds418j_1.hbk\Pool\0\17\0.bucket",
            &[5, 17, 23, 29, 35, 41, 53, 59, 65, 67, 75, 83, 89, 95, 100],
        );
        push_new_file(
            &mut expected,
            Information::new::<byte>(204_576),
            r"J:\nas-ds418j_1.hbk\Pool\0\17\0.index",
            &[100],
        );
        push_new_file(
            &mut expected,
            Information::new::<byte>(0),
            r"J:\nas-ds418j_1.hbk\Pool\0\17\0.lock",
            &[100],
        );
        push_new_file(
            &mut expected,
            Information::new::<mebibyte>(50),
            r"J:\nas-ds418j_1.hbk\Pool\0\17\1.bucket",
            &[91, 97, 100],
        );
        push_new_file(
            &mut expected,
            Information::new::<byte>(204_224),
            r"J:\nas-ds418j_1.hbk\Pool\0\17\1.index",
            &[100],
        );
        push_new_file(
            &mut expected,
            Information::new::<byte>(0),
            r"J:\nas-ds418j_1.hbk\Pool\0\17\1.lock",
            &[100],
        );
        push_new_file(
            &mut expected,
            Information::new::<mebibyte>(50),
            r"J:\nas-ds418j_1.hbk\Pool\0\17\10.bucket",
            &[],
        );
        assert_eq!(entries, expected, "Parsed entries mismatch");
        Ok(())
    }
}
