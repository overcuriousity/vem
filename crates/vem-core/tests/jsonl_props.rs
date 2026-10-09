use proptest::prelude::*;
use std::io::Cursor;
use vem_core::jsonl::JsonlReader;

fn line_strategy() -> impl Strategy<Value = Vec<u8>> {
    // Bytes that are not '\n' and not '\r', so each generated line is exactly one record.
    prop::collection::vec(any::<u8>().prop_filter("no newline", |b| *b != b'\n' && *b != b'\r'), 0..40)
}

proptest! {
    #[test]
    fn offsets_index_back_into_the_original_bytes(
        lines in prop::collection::vec(line_strategy(), 0..12),
        trailing_newline in any::<bool>(),
    ) {
        let mut data = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            data.extend_from_slice(l);
            if i + 1 < lines.len() || trailing_newline {
                data.push(b'\n');
            }
        }
        let recs: Vec<_> = JsonlReader::new(Cursor::new(data.clone())).map(|r| r.unwrap()).collect();
        let non_empty: Vec<_> = lines.iter().enumerate().filter(|(_, l)| !l.is_empty()).collect();
        prop_assert_eq!(recs.len(), non_empty.len());
        for (rec, (idx, line)) in recs.iter().zip(non_empty.iter()) {
            prop_assert_eq!(rec.index as usize, *idx);
            prop_assert_eq!(&rec.bytes, *line);
            let slice = &data[rec.offset as usize..(rec.offset + rec.length) as usize];
            prop_assert_eq!(slice, line.as_slice());
        }
        if let Some(last) = recs.last() {
            let last_line_is_final = non_empty.last().map(|(i, _)| *i + 1 == lines.len()).unwrap_or(false);
            prop_assert_eq!(last.terminated, !(last_line_is_final && !trailing_newline));
        }
    }
}
