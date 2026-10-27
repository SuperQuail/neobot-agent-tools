use crate::core::SnapshotData;

#[test]
fn empty_text_has_no_lines() {
    assert_eq!(SnapshotData::create(b"").unwrap().line_count(), 0);
}

#[test]
fn unicode_and_mixed_newlines_keep_python_boundaries() {
    let value = SnapshotData::create("汉😀\r\na\rb\u{85}tail".as_bytes()).unwrap();
    assert_eq!(
        (0..value.line_count())
            .map(|index| value.line(index, false).unwrap())
            .collect::<Vec<_>>(),
        ["汉😀", "a", "b", "tail"]
    );
}

#[test]
fn unicode_checkpoint_in_crlf_keeps_character_offsets() {
    let text = format!("{}\r\n😀tail", "汉".repeat(255));
    let value = SnapshotData::create(text.as_bytes()).unwrap();
    assert_eq!(value.fragment(1, 1, 4).unwrap(), ("tail".to_owned(), 4));
}

#[test]
fn long_unicode_line_page_respects_byte_budget() {
    let value = SnapshotData::create("汉😀汉\n".as_bytes()).unwrap();
    let page = value.page(1, 1, 0, 6).unwrap();
    assert_eq!(
        (page.0, page.3, page.4),
        (vec![(1, "汉".to_owned(), 0)], Some(1), Some(1))
    );
}

#[test]
fn normalization_matches_all_python_line_separators() {
    let value = SnapshotData::create(
        "a\r\nb\rc\u{b}d\u{c}e\u{1c}f\u{1d}g\u{1e}h\u{85}i\u{2028}j\u{2029}".as_bytes(),
    )
    .unwrap();
    assert_eq!(
        value.normalized().unwrap(),
        b"a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n"
    );
}

#[test]
fn invalid_utf8_is_rejected() {
    assert!(SnapshotData::create(&[0xff]).is_err());
}

#[test]
fn invalid_page_limits_are_rejected() {
    assert!(
        SnapshotData::create(b"text")
            .unwrap()
            .page(1, 2001, 0, 65536)
            .is_err()
    );
}
