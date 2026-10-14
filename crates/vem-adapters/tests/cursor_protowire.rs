//! The schemaless protobuf wire walker: strict parsing, string extraction, 32-byte refs, and caps on
//! arbitrary input.

use proptest::collection::vec;
use proptest::prelude::*;
use vem_adapters::cursor::protowire::{
    extract_strings, parse_message, refs, Field, WireError, WireValue, OUTPUT_CAP,
};

fn varint(mut v: u64, out: &mut Vec<u8>) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn bytes_field(n: u64, data: &[u8], out: &mut Vec<u8>) {
    varint(n << 3 | 2, out);
    varint(data.len() as u64, out);
    out.extend_from_slice(data);
}

#[test]
fn parses_varint_and_nested() {
    // field 1 varint 150 (08 96 01); field 2 bytes = { field 1 varint 1 }; field 3 fixed32; field 4 fixed64.
    let mut b = vec![0x08, 0x96, 0x01, 0x12, 0x02, 0x08, 0x01];
    b.extend_from_slice(&[0x1d, 1, 0, 0, 0]);
    b.extend_from_slice(&[0x21, 2, 0, 0, 0, 0, 0, 0, 0]);
    let fields = parse_message(&b).unwrap();
    assert_eq!(
        fields,
        vec![
            Field {
                number: 1,
                value: WireValue::Varint(150)
            },
            Field {
                number: 2,
                value: WireValue::Bytes(vec![0x08, 0x01])
            },
            Field {
                number: 3,
                value: WireValue::Fixed32(1)
            },
            Field {
                number: 4,
                value: WireValue::Fixed64(2)
            },
        ]
    );
    let WireValue::Bytes(inner) = &fields[1].value else {
        unreachable!()
    };
    assert_eq!(
        parse_message(inner).unwrap(),
        vec![Field {
            number: 1,
            value: WireValue::Varint(1)
        }]
    );
}

#[test]
fn rejects_truncated_length() {
    // field 1, length 5, only 2 bytes follow.
    assert!(matches!(
        parse_message(&[0x0a, 0x05, b'a', b'b']),
        Err(WireError::Truncated)
    ));
    // A varint cut off mid-way, and a fixed64 short of 8 bytes.
    assert!(parse_message(&[0x08, 0x96]).is_err());
    assert!(parse_message(&[0x09, 1, 2, 3]).is_err());
}

#[test]
fn rejects_group_wire_types() {
    for wt in [3u8, 4, 6, 7] {
        let tag = (1 << 3) | wt;
        assert!(
            matches!(parse_message(&[tag, 0]), Err(WireError::BadWireType(w)) if w == wt),
            "wire type {wt}"
        );
    }
    // Field number 0 and an over-long (11 byte) varint.
    assert!(matches!(
        parse_message(&[0x00, 0x00]),
        Err(WireError::FieldZero)
    ));
    let mut long = vec![0x08];
    long.extend_from_slice(&[0xff; 10]);
    long.push(0x01);
    assert!(matches!(
        parse_message(&long),
        Err(WireError::VarintTooLong)
    ));
}

#[test]
fn extract_strings_finds_nested_text() {
    let mut inner = Vec::new();
    bytes_field(1, "list the files".as_bytes(), &mut inner);
    bytes_field(2, &[0x00, 0x01, 0x02, 0xff], &mut inner); // binary: not a string, not a message
    let mut mid = Vec::new();
    bytes_field(5, &inner, &mut mid);
    let mut outer = vec![0x08, 0x01];
    bytes_field(3, "héllo wörld".as_bytes(), &mut outer);
    bytes_field(4, &mid, &mut outer);
    bytes_field(6, b"x", &mut outer); // shorter than 2: not a string
    assert_eq!(
        extract_strings(&outer, 8),
        vec!["héllo wörld".to_string(), "list the files".to_string()]
    );
    // Depth limit: with max depth 1 the doubly nested string is not reached.
    assert_eq!(extract_strings(&outer, 1), vec!["héllo wörld".to_string()]);
    // Not a message at all.
    assert!(extract_strings(b"{\"role\":\"user\"}", 8).is_empty());
}

#[test]
fn refs_finds_32_byte_values() {
    let a = [0xaa; 32];
    let c = [0xcc; 32];
    let mut inner = Vec::new();
    bytes_field(1, &c, &mut inner);
    let mut b = Vec::new();
    bytes_field(1, &a, &mut b);
    bytes_field(2, &[0xbb; 31], &mut b); // not 32 bytes
    bytes_field(3, &inner, &mut b);
    assert_eq!(refs(&b), vec![a, c]);
    assert!(refs(&[0xff, 0xff]).is_empty());
}

proptest! {
    #[test]
    fn walk_never_panics(bytes in vec(any::<u8>(), 0..4096)) {
        let _ = parse_message(&bytes);
        let strings = extract_strings(&bytes, 8);
        prop_assert!(strings.iter().map(String::len).sum::<usize>() <= OUTPUT_CAP);
        let r = refs(&bytes);
        prop_assert!(r.len() * 32 <= OUTPUT_CAP);
    }
}
