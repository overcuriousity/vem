//! A schemaless protobuf wire walker for Cursor's `store.db` blobs (spec §6.3). Nothing about the
//! messages is known, so the walk only reads the wire format: strictly, without recursion on the call
//! stack beyond `MAX_DEPTH`, and within fixed caps on fields visited and bytes returned.

pub const MAX_DEPTH: u8 = 8;
/// Fields visited per walk (`extract_strings`, `refs`), over all depths.
pub const FIELD_CAP: usize = 100_000;
/// Bytes returned per walk.
pub const OUTPUT_CAP: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireValue {
    Varint(u64),
    Fixed64(u64),
    Fixed32(u32),
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub number: u64,
    pub value: WireValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// A varint, fixed value or length-delimited value runs past the end.
    Truncated,
    /// A varint longer than 10 bytes.
    VarintTooLong,
    /// Wire types 3 and 4 (groups) and 6, 7 (undefined).
    BadWireType(u8),
    FieldZero,
}

/// A field whose `Bytes` value borrows from the input.
enum Raw<'a> {
    Varint(u64),
    Fixed64(u64),
    Fixed32(u32),
    Bytes(&'a [u8]),
}

fn varint(b: &[u8], pos: &mut usize) -> Result<u64, WireError> {
    let mut v = 0u64;
    for i in 0..10 {
        let byte = *b.get(*pos).ok_or(WireError::Truncated)?;
        *pos += 1;
        v |= u64::from(byte & 0x7f) << (7 * i);
        if byte & 0x80 == 0 {
            return Ok(v);
        }
    }
    Err(WireError::VarintTooLong)
}

fn take<'a>(b: &'a [u8], pos: &mut usize, n: u64) -> Result<&'a [u8], WireError> {
    let rest = &b[*pos..];
    if n > rest.len() as u64 {
        return Err(WireError::Truncated);
    }
    let n = n as usize;
    *pos += n;
    Ok(&rest[..n])
}

/// The next field at `pos`, or `None` at the end of `b`.
fn next<'a>(b: &'a [u8], pos: &mut usize) -> Result<Option<(u64, Raw<'a>)>, WireError> {
    if *pos >= b.len() {
        return Ok(None);
    }
    let tag = varint(b, pos)?;
    let number = tag >> 3;
    let value = match (tag & 7) as u8 {
        0 => Raw::Varint(varint(b, pos)?),
        1 => {
            let s = take(b, pos, 8)?;
            Raw::Fixed64(u64::from_le_bytes(s.try_into().expect("8 bytes")))
        }
        2 => {
            let n = varint(b, pos)?;
            Raw::Bytes(take(b, pos, n)?)
        }
        5 => {
            let s = take(b, pos, 4)?;
            Raw::Fixed32(u32::from_le_bytes(s.try_into().expect("4 bytes")))
        }
        wt => return Err(WireError::BadWireType(wt)),
    };
    if number == 0 {
        return Err(WireError::FieldZero);
    }
    Ok(Some((number, value)))
}

/// The fields of `b`, borrowed; fails on the first wire error.
fn raw_fields(b: &[u8]) -> Result<Vec<(u64, Raw<'_>)>, WireError> {
    let mut pos = 0;
    let mut out = Vec::new();
    while let Some(f) = next(b, &mut pos)? {
        out.push(f);
    }
    Ok(out)
}

/// The top-level fields of one message. Strict: any wire error fails the whole message.
pub fn parse_message(b: &[u8]) -> Result<Vec<Field>, WireError> {
    Ok(raw_fields(b)?
        .into_iter()
        .map(|(number, v)| Field {
            number,
            value: match v {
                Raw::Varint(x) => WireValue::Varint(x),
                Raw::Fixed64(x) => WireValue::Fixed64(x),
                Raw::Fixed32(x) => WireValue::Fixed32(x),
                Raw::Bytes(x) => WireValue::Bytes(x.to_vec()),
            },
        })
        .collect())
}

/// Text a human would read: valid UTF-8, at least 2 bytes, and at least 90 % printable characters
/// (whitespace counts as printable).
fn as_text(b: &[u8]) -> Option<&str> {
    if b.len() < 2 {
        return None;
    }
    let s = std::str::from_utf8(b).ok()?;
    let (mut total, mut printable) = (0usize, 0usize);
    for c in s.chars() {
        total += 1;
        if !c.is_control() || c.is_whitespace() {
            printable += 1;
        }
    }
    (printable * 10 >= total * 9).then_some(s)
}

/// Whether all of `b` is well-formed wire data (without allocating).
fn valid(b: &[u8]) -> bool {
    let mut pos = 0;
    loop {
        match next(b, &mut pos) {
            Ok(Some(_)) => {}
            Ok(None) => return true,
            Err(_) => return false,
        }
    }
}

/// Depth-first walk over every `Bytes` value of `b`, in document order. `visit` returns whether to
/// descend into a value (done when it is well-formed wire data and the depth allows); `None` stops the
/// walk. Frames are cursors into the input, so memory is bounded by the depth, not the field count.
fn walk<'a>(b: &'a [u8], max_depth: u8, mut visit: impl FnMut(&'a [u8]) -> Option<bool>) {
    let max_depth = max_depth.min(MAX_DEPTH);
    if !valid(b) {
        return;
    }
    let mut fields = 0usize;
    let mut stack: Vec<(u8, &'a [u8], usize)> = vec![(0, b, 0)];
    while let Some((depth, msg, pos)) = stack.last_mut() {
        let (depth, msg) = (*depth, *msg);
        // Every frame was validated before it was pushed.
        let Ok(Some((_, value))) = next(msg, pos) else {
            stack.pop();
            continue;
        };
        fields += 1;
        if fields > FIELD_CAP {
            return;
        }
        let Raw::Bytes(v) = value else { continue };
        match visit(v) {
            None => return,
            Some(true) if depth + 1 < max_depth && valid(v) => stack.push((depth + 1, v, 0)),
            Some(_) => {}
        }
    }
}

/// Every human-readable string at any depth, in document order. A value that is text is not also
/// walked as a message. Caps: depth `min(max_depth, 8)`, 100 000 fields, 1 MiB of output (a string that
/// would cross the cap is cut at a character boundary and ends the walk).
pub fn extract_strings(b: &[u8], max_depth: u8) -> Vec<String> {
    let mut out = Vec::new();
    let mut size = 0usize;
    walk(b, max_depth, |v| {
        let Some(s) = as_text(v) else {
            return Some(true);
        };
        let room = OUTPUT_CAP - size;
        if s.len() <= room {
            size += s.len();
            out.push(s.to_string());
            return Some(false);
        }
        let mut cut = room;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        if cut > 0 {
            out.push(s[..cut].to_string());
        }
        None
    });
    out
}

/// Every 32-byte `Bytes` value at any depth (a blob reference by its SHA-256), in document order. A
/// reference is not walked as a message.
pub fn refs(b: &[u8]) -> Vec<[u8; 32]> {
    let mut out = Vec::new();
    walk(b, MAX_DEPTH, |v| match <[u8; 32]>::try_from(v) {
        Ok(r) if (out.len() + 1) * 32 <= OUTPUT_CAP => {
            out.push(r);
            Some(false)
        }
        Ok(_) => None,
        Err(_) => Some(true),
    });
    out
}

/// Whether `b` is a tree node: a non-empty message whose every field is a reference or a varint.
pub fn is_ref_node(b: &[u8]) -> bool {
    let mut pos = 0;
    let mut any = false;
    loop {
        match next(b, &mut pos) {
            Ok(None) => return any,
            Ok(Some((_, Raw::Varint(_)))) => {}
            Ok(Some((_, Raw::Bytes(x)))) if x.len() == 32 => {}
            _ => return false,
        }
        any = true;
    }
}
