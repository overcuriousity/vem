use vem_case::diff::*;

#[test]
fn creation_is_all_inserts() {
    let r = diff_bytes(b"", b"a\nb\n");
    assert!(!r.binary);
    assert_eq!(r.hunks.len(), 1);
    let tags: Vec<Tag> = r.hunks[0].lines.iter().map(|l| l.tag).collect();
    assert_eq!(tags, vec![Tag::Insert, Tag::Insert]);
    assert_eq!(r.hunks[0].lines[1].new_no, Some(2));
    assert_eq!(r.hunks[0].lines[1].old_no, None);
    assert_eq!(r.hunks[0].lines[0].text, "a");
}

#[test]
fn edit_has_context_and_line_numbers() {
    let old: String = (1..=20).map(|i| format!("line {i}\n")).collect();
    let new = old.replace("line 10\n", "line ten\n");
    let r = diff_bytes(old.as_bytes(), new.as_bytes());
    assert_eq!(r.hunks.len(), 1);
    let h = &r.hunks[0];
    assert_eq!(
        (h.old_start, h.old_lines, h.new_start, h.new_lines),
        (7, 7, 7, 7)
    );
    let del = h.lines.iter().find(|l| l.tag == Tag::Delete).unwrap();
    assert_eq!((del.old_no, del.text.as_str()), (Some(10), "line 10"));
    let ins = h.lines.iter().find(|l| l.tag == Tag::Insert).unwrap();
    assert_eq!((ins.new_no, ins.text.as_str()), (Some(10), "line ten"));
    assert_eq!(
        h.lines.iter().filter(|l| l.tag == Tag::Equal).count(),
        2 * CONTEXT
    );
}

#[test]
fn identical_is_empty_and_binary_is_detected() {
    assert!(diff_bytes(b"same\n", b"same\n").hunks.is_empty());
    for bin in [&b"a\0b"[..], &[0xff, 0xfe, 0x41][..]] {
        let r = diff_bytes(b"text\n", bin);
        assert!(r.binary && r.hunks.is_empty());
        assert!(is_binary(bin));
    }
    assert!(!is_binary("héllo".as_bytes()));
    // A UTF-8 sequence cut at the end (truncated read) is still text.
    assert!(!is_binary(&"é".as_bytes()[..1]));
    assert!(serde_json::to_value(Tag::Insert).unwrap() == "insert");
}
