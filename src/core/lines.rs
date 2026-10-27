//! 统一处理 Python splitlines 换行语义，以及检索所需的 UTF-8 换行归一化。

use super::{CoreError, reserved};

pub(super) fn is_break(character: char) -> bool {
    matches!(
        character,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

pub(super) fn has_standard_breaks(data: &[u8]) -> bool {
    let has_ascii_separator = memchr::memchr3(0x0b, 0x0c, 0x1c, data).is_some()
        || memchr::memchr2(0x1d, 0x1e, data).is_some();
    if has_ascii_separator {
        return false;
    }
    ["\u{85}", "\u{2028}", "\u{2029}"]
        .iter()
        .all(|separator| memchr::memmem::find(data, separator.as_bytes()).is_none())
}

/// 返回 CR、LF 或 CRLF 的结束字节位置；CRLF 只产生一个行边界。
fn standard_line_ends(data: &[u8]) -> impl Iterator<Item = usize> + '_ {
    memchr::memchr2_iter(b'\r', b'\n', data)
        .filter(|&byte| {
            let is_crlf_tail = data[byte] == b'\n' && byte > 0 && data[byte - 1] == b'\r';
            !is_crlf_tail
        })
        .map(|byte| {
            let is_crlf = data[byte] == b'\r' && data.get(byte + 1) == Some(&b'\n');
            byte + 1 + usize::from(is_crlf)
        })
}

/// 扫描换行结束位置及累计字符数，不为每个字符执行回调。
pub(super) fn scan_lines(
    text: &str,
    standard: bool,
    mut on_line: impl FnMut(usize, usize),
) -> usize {
    if !standard {
        return scan_unicode_lines(text, on_line);
    }
    let mut previous_byte = 0;
    let mut characters = 0;
    for end_byte in standard_line_ends(text.as_bytes()) {
        characters += text[previous_byte..end_byte].chars().count();
        on_line(end_byte, characters);
        previous_byte = end_byte;
    }
    characters + text[previous_byte..].chars().count()
}

fn scan_unicode_lines(text: &str, mut on_line: impl FnMut(usize, usize)) -> usize {
    let mut iterator = text.char_indices().peekable();
    let mut characters = 0;
    while let Some((byte, character)) = iterator.next() {
        characters += 1;
        let mut end_byte = byte + character.len_utf8();
        let is_crlf = character == '\r' && iterator.peek().is_some_and(|(_, next)| *next == '\n');
        if is_crlf {
            // 已通过 peek 确认下一字符是 LF；CRLF 占两个字符，但只结束一行。
            iterator.next();
            characters += 1;
            end_byte += 1;
        }
        if is_break(character) {
            on_line(end_byte, characters);
        }
    }
    characters
}

/// 将范围内的字节位置向前收敛到 UTF-8 字符边界，最多回退三个字节。
pub(super) fn floor_char_boundary(text: &str, mut byte: usize) -> usize {
    while !text.is_char_boundary(byte) {
        byte -= 1;
    }
    byte
}

pub(super) fn normalized(text: &str, standard: bool) -> Result<Vec<u8>, CoreError> {
    let mut output = reserved(text.len() + 1)?;
    if standard {
        normalize_standard(text.as_bytes(), &mut output);
    } else {
        normalize_unicode(text, &mut output);
    }
    if !output.is_empty() && output.last() != Some(&b'\n') {
        output.push(b'\n');
    }
    Ok(output)
}

fn normalize_standard(data: &[u8], output: &mut Vec<u8>) {
    let mut copied_until = 0;
    for end_byte in standard_line_ends(data) {
        let newline_bytes = if data[..end_byte].ends_with(b"\r\n") {
            2
        } else {
            1
        };
        let start_byte = end_byte - newline_bytes;
        output.extend_from_slice(&data[copied_until..start_byte]);
        output.push(b'\n');
        copied_until = end_byte;
    }
    output.extend_from_slice(&data[copied_until..]);
}

fn normalize_unicode(text: &str, output: &mut Vec<u8>) {
    let mut copied_until = 0;
    let mut iterator = text.char_indices().peekable();
    while let Some((start_byte, character)) = iterator.next() {
        if !is_break(character) {
            continue;
        }
        output.extend_from_slice(&text.as_bytes()[copied_until..start_byte]);
        output.push(b'\n');
        copied_until = start_byte + character.len_utf8();
        let is_crlf = character == '\r' && iterator.peek().is_some_and(|(_, next)| *next == '\n');
        if is_crlf {
            iterator.next();
            copied_until += 1;
        }
    }
    output.extend_from_slice(&text.as_bytes()[copied_until..]);
}
