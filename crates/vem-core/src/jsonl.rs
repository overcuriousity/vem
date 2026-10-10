//! Tolerant JSONL reader: byte offsets for provenance, truncated last line, CRLF, blank lines,
//! and an upper bound on record size so a corrupt file cannot exhaust memory.

use std::io::{self, BufRead};

/// One physical line of a JSONL file, with the byte range it occupies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRecord {
    /// 0-based physical line number (blank lines count).
    pub index: u64,
    /// Byte offset of the first byte of the line.
    pub offset: u64,
    /// Length of the line on disk excluding the newline and a trailing `\r`.
    pub length: u64,
    /// Line content without newline or trailing `\r`; capped at `max_len` when `oversized`.
    pub bytes: Vec<u8>,
    /// `false` when the file ended without a newline after this line.
    pub terminated: bool,
    /// `true` when the line exceeded `max_len`; `bytes` holds only the first `max_len` bytes.
    pub oversized: bool,
}

pub const DEFAULT_MAX_LEN: usize = 64 * 1024 * 1024;

pub struct JsonlReader<R: BufRead> {
    inner: R,
    offset: u64,
    index: u64,
    max_len: usize,
    done: bool,
}

impl<R: BufRead> JsonlReader<R> {
    pub fn new(inner: R) -> Self {
        Self::with_max_len(inner, DEFAULT_MAX_LEN)
    }

    pub fn with_max_len(inner: R, max_len: usize) -> Self {
        Self {
            inner,
            offset: 0,
            index: 0,
            max_len,
            done: false,
        }
    }

    /// Reads one physical line. Returns `Ok(None)` at EOF.
    fn read_line(&mut self) -> io::Result<Option<RawRecord>> {
        let start = self.offset;
        let mut bytes: Vec<u8> = Vec::new();
        let mut consumed: u64 = 0;
        let mut oversized = false;
        let mut terminated = false;
        let mut last_byte: Option<u8> = None;
        loop {
            let buf = self.inner.fill_buf()?;
            if buf.is_empty() {
                break;
            }
            let (chunk, used, hit_newline) = match buf.iter().position(|&b| b == b'\n') {
                Some(pos) => (&buf[..pos], pos + 1, true),
                None => (buf, buf.len(), false),
            };
            if let Some(&b) = chunk.last() {
                last_byte = Some(b);
            }
            let room = self.max_len.saturating_sub(bytes.len());
            if chunk.len() > room {
                bytes.extend_from_slice(&chunk[..room]);
                oversized = true;
            } else {
                bytes.extend_from_slice(chunk);
            }
            consumed += used as u64;
            self.inner.consume(used);
            if hit_newline {
                terminated = true;
                break;
            }
        }
        if consumed == 0 {
            return Ok(None);
        }
        self.offset += consumed;
        let index = self.index;
        self.index += 1;
        let mut length = consumed - if terminated { 1 } else { 0 };
        if last_byte == Some(b'\r') {
            length = length.saturating_sub(1);
            if bytes.last() == Some(&b'\r') {
                bytes.pop();
            }
        }
        Ok(Some(RawRecord {
            index,
            offset: start,
            length,
            bytes,
            terminated,
            oversized,
        }))
    }
}

impl<R: BufRead> Iterator for JsonlReader<R> {
    type Item = io::Result<RawRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.done {
                return None;
            }
            match self.read_line() {
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
                Ok(None) => {
                    self.done = true;
                    return None;
                }
                Ok(Some(rec)) => {
                    if !rec.terminated {
                        self.done = true;
                    }
                    if rec.length == 0 {
                        continue;
                    }
                    return Some(Ok(rec));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn read_all(data: &[u8], cap: usize) -> Vec<RawRecord> {
        JsonlReader::with_max_len(Cursor::new(data.to_vec()), cap)
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn yields_offsets_and_lengths_for_terminated_lines() {
        let recs = read_all(b"{\"a\":1}\n{\"b\":22}\n", 1024);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].index, 0);
        assert_eq!(recs[0].offset, 0);
        assert_eq!(recs[0].length, 7);
        assert_eq!(recs[0].bytes, b"{\"a\":1}");
        assert!(recs[0].terminated);
        assert_eq!(recs[1].index, 1);
        assert_eq!(recs[1].offset, 8);
        assert_eq!(recs[1].length, 8);
        assert!(recs[1].terminated);
    }

    #[test]
    fn last_line_without_newline_is_unterminated() {
        let recs = read_all(b"{\"a\":1}\n{\"b\":2", 1024);
        assert_eq!(recs.len(), 2);
        assert!(!recs[1].terminated);
        assert_eq!(recs[1].bytes, b"{\"b\":2");
        assert_eq!(recs[1].length, 6);
    }

    #[test]
    fn strips_carriage_return_and_counts_it_in_length() {
        let recs = read_all(b"{\"a\":1}\r\n{\"b\":2}\r\n", 1024);
        assert_eq!(recs[0].bytes, b"{\"a\":1}");
        assert_eq!(recs[0].length, 7);
        assert_eq!(recs[1].offset, 9);
    }

    #[test]
    fn skips_blank_lines_but_keeps_line_index() {
        let recs = read_all(b"{\"a\":1}\n\n\n{\"b\":2}\n", 1024);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[1].index, 3);
        assert_eq!(recs[1].offset, 10);
    }

    #[test]
    fn caps_oversized_lines_and_keeps_subsequent_offsets_right() {
        let recs = read_all(b"abcdefgh\n{}\n", 4);
        assert_eq!(recs.len(), 2);
        assert!(recs[0].oversized);
        assert_eq!(recs[0].bytes, b"abcd");
        assert_eq!(recs[0].length, 8);
        assert!(!recs[1].oversized);
        assert_eq!(recs[1].offset, 9);
        assert_eq!(recs[1].bytes, b"{}");
    }

    #[test]
    fn empty_input_yields_nothing() {
        assert!(read_all(b"", 1024).is_empty());
        assert!(read_all(b"\n\n", 1024).is_empty());
    }
}
