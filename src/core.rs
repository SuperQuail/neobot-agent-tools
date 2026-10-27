//! File-system independent UTF-8 snapshots with Python-compatible line boundaries.
use std::sync::Arc;

pub(crate) const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_TEXT_BYTES: usize = 65536;
const CHECKPOINT_BYTES: usize = 256;

#[derive(Debug, thiserror::Error)]
pub(crate) enum CoreError {
    #[error("Maximum input size is 16777216 bytes")]
    TooLarge,
    #[error("Invalid UTF-8")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("Could not allocate the bounded snapshot")]
    Allocation,
    #[error("Line index is outside the snapshot")]
    LineIndex,
    #[error("{0}")]
    Argument(&'static str),
}

#[derive(Clone, Copy)]
struct Position {
    byte: u32,
    character: u32,
}

pub(crate) struct SnapshotData {
    text: Box<str>,
    positions: Vec<Position>,
    checkpoints: Vec<Position>,
    characters: usize,
    ascii: bool,
    standard_breaks: bool,
}

pub(crate) type Row = (usize, String, usize);
pub(crate) type Page = (Vec<Row>, usize, usize, Option<usize>, Option<usize>);

fn is_break(ch: char) -> bool {
    matches!(
        ch,
        '\n' | '\r' | '\u{b}' | '\u{c}' | '\u{1c}'..='\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

fn scan(
    text: &str,
    mut character: impl FnMut(usize, usize),
    mut line: impl FnMut(usize, usize),
) -> usize {
    let mut iterator = text.char_indices().peekable();
    let mut count = 0;
    while let Some((byte, ch)) = iterator.next() {
        character(byte, count);
        count += 1;
        let mut end = byte + ch.len_utf8();
        if ch == '\r' && iterator.peek().is_some_and(|(_, next)| *next == '\n') {
            if let Some((byte, _)) = iterator.next() {
                character(byte, count);
                count += 1;
                end = byte + 1;
            }
        }
        if is_break(ch) {
            line(end, count);
        }
    }
    count
}

fn reserved<T>(count: usize) -> Result<Vec<T>, CoreError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| CoreError::Allocation)?;
    Ok(values)
}

fn standard_ends(data: &[u8]) -> impl Iterator<Item = usize> + '_ {
    memchr::memchr2_iter(b'\r', b'\n', data)
        .filter(|&byte| data[byte] != b'\n' || byte == 0 || data[byte - 1] != b'\r')
        .map(|byte| {
            byte + 1 + usize::from(data[byte] == b'\r' && data.get(byte + 1) == Some(&b'\n'))
        })
}

fn scan_lines(text: &str, standard: bool, mut line: impl FnMut(usize, usize)) -> usize {
    if !standard {
        return scan(text, |_, _| {}, line);
    }
    let (mut previous, mut characters) = (0, 0);
    for end in standard_ends(text.as_bytes()) {
        characters += text[previous..end].chars().count();
        line(end, characters);
        previous = end;
    }
    characters + text[previous..].chars().count()
}

impl SnapshotData {
    pub(crate) fn create(data: &[u8]) -> Result<Arc<Self>, CoreError> {
        if data.len() > MAX_FILE_BYTES {
            return Err(CoreError::TooLarge);
        }
        let text = std::str::from_utf8(data)?;
        let standard_breaks = memchr::memchr3(0x0b, 0x0c, 0x1c, data).is_none()
            && memchr::memchr2(0x1d, 0x1e, data).is_none()
            && ["\u{85}", "\u{2028}", "\u{2029}"]
                .iter()
                .all(|separator| memchr::memmem::find(data, separator.as_bytes()).is_none());
        let mut boundaries = 0;
        let mut last = 0;
        let characters = scan_lines(text, standard_breaks, |byte, _| {
            boundaries += 1;
            last = byte;
        });
        let tail = last < text.len();
        // Exact reservation prevents dense newline inputs from doubling a large
        // index allocation while the old allocation is still live.
        let mut positions = reserved(boundaries + usize::from(tail) + 1)?;
        positions.push(Position {
            byte: 0,
            character: 0,
        });
        let ascii = text.is_ascii();
        let mut checkpoints = reserved(if ascii {
            0
        } else {
            data.len().div_ceil(CHECKPOINT_BYTES - 3)
        })?;
        scan_lines(text, standard_breaks, |byte, character| {
            positions.push(Position {
                byte: byte as u32,
                character: character as u32,
            });
        });
        if !ascii {
            let (mut byte, mut character) = (0, 0);
            while byte < data.len() {
                checkpoints.push(Position {
                    byte: byte as u32,
                    character: character as u32,
                });
                let mut end = (byte + CHECKPOINT_BYTES).min(data.len());
                while !text.is_char_boundary(end) {
                    end -= 1;
                }
                character += text[byte..end].chars().count();
                byte = end;
            }
        }
        if tail {
            positions.push(Position {
                byte: data.len() as u32,
                character: characters as u32,
            });
        }
        let mut owned = reserved(data.len())?;
        owned.extend_from_slice(data);
        let text = String::from_utf8(owned)
            .map_err(|error| CoreError::Utf8(error.utf8_error()))?
            .into_boxed_str();
        Ok(Arc::new(Self {
            text,
            positions,
            checkpoints,
            characters,
            ascii,
            standard_breaks,
        }))
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }
    pub(crate) fn size(&self) -> usize {
        self.text.len()
    }
    pub(crate) fn characters(&self) -> usize {
        self.characters
    }
    pub(crate) fn line_count(&self) -> usize {
        self.positions.len() - 1
    }
    pub(crate) fn memory_size(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.text.len()
            + self.positions.capacity() * std::mem::size_of::<Position>()
            + self.checkpoints.capacity() * std::mem::size_of::<Position>()
    }
    pub(crate) fn start(&self, index: usize) -> usize {
        self.positions
            .get(index)
            .map_or(self.characters, |position| position.character as usize)
    }

    fn ends(&self, index: usize) -> Result<(Position, Position), CoreError> {
        let next = index.checked_add(1).ok_or(CoreError::LineIndex)?;
        match (self.positions.get(index), self.positions.get(next)) {
            (Some(start), Some(end)) => Ok((*start, *end)),
            _ => Err(CoreError::LineIndex),
        }
    }

    fn byte_for_character(&self, character: usize) -> Result<usize, CoreError> {
        if character > self.characters {
            return Err(CoreError::Argument("column exceeds line length"));
        }
        if self.ascii || character == self.characters {
            return Ok(if self.ascii {
                character
            } else {
                self.text.len()
            });
        }
        let checkpoint_index = self
            .checkpoints
            .partition_point(|p| p.character as usize <= character);
        let checkpoint = self
            .checkpoints
            .get(checkpoint_index.saturating_sub(1))
            .ok_or(CoreError::LineIndex)?;
        let start = checkpoint.byte as usize;
        let suffix = self.text.get(start..).ok_or(CoreError::LineIndex)?;
        suffix
            .char_indices()
            .nth(character - checkpoint.character as usize)
            .map(|(offset, _)| start + offset)
            .ok_or(CoreError::LineIndex)
    }

    pub(crate) fn line(&self, index: usize, keepends: bool) -> Result<&str, CoreError> {
        let (start, end) = self.ends(index)?;
        let line = self
            .text
            .get(start.byte as usize..end.byte as usize)
            .ok_or(CoreError::LineIndex)?;
        if !keepends {
            if let Some(without) = line.strip_suffix("\r\n") {
                return Ok(without);
            }
            if let Some(last) = line.chars().next_back().filter(|ch| is_break(*ch)) {
                return Ok(&line[..line.len() - last.len_utf8()]);
            }
        }
        Ok(line)
    }

    fn fragment_bounds(
        &self,
        index: usize,
        column: usize,
        limit: usize,
    ) -> Result<(usize, usize, usize), CoreError> {
        let (start, end) = self.ends(index)?;
        let length = (end.character - start.character) as usize;
        if column > length {
            return Err(CoreError::Argument("column exceeds line length"));
        }
        let character = start.character as usize + column;
        let remaining = length - column;
        let first = if column == 0 {
            start.byte as usize
        } else {
            self.byte_for_character(character)?
        };
        let last = if limit >= remaining {
            end.byte as usize
        } else {
            self.byte_for_character(character + limit)?
        };
        Ok((first, last, remaining))
    }

    pub(crate) fn fragment(
        &self,
        index: usize,
        column: usize,
        limit: usize,
    ) -> Result<(String, usize), CoreError> {
        if limit > MAX_TEXT_BYTES {
            return Err(CoreError::Argument("char_limit exceeds 65536"));
        }
        let (start, end, remaining) = self.fragment_bounds(index, column, limit)?;
        Ok((self.text[start..end].to_owned(), remaining))
    }

    pub(crate) fn page(
        &self,
        offset: usize,
        limit: usize,
        column: usize,
        budget: usize,
    ) -> Result<Page, CoreError> {
        if offset == 0
            || offset > MAX_FILE_BYTES + 1
            || limit == 0
            || limit > 2000
            || column > MAX_FILE_BYTES
            || budget > MAX_TEXT_BYTES
        {
            return Err(CoreError::Argument(
                "Invalid read offset, limit, column or byte budget",
            ));
        }
        if column > 0
            && (offset > self.line_count() || column > self.start(offset) - self.start(offset - 1))
        {
            return Err(CoreError::Argument("column exceeds line length"));
        }
        let first = self.start(offset - 1) + column;
        let (mut position, mut remaining) = (first, budget);
        let mut rows = Vec::new();
        let (mut next_offset, mut next_column) = (None, None);
        for index in offset - 1..self.line_count().min(offset - 1 + limit) {
            let begin = if index == offset - 1 { column } else { 0 };
            let (start, end, line_chars) = self.fragment_bounds(index, begin, remaining)?;
            let mut stop = end.min(start + remaining);
            while !self.text.is_char_boundary(stop) {
                stop -= 1;
            }
            let value = &self.text[start..stop];
            if value.is_empty() && line_chars > 0 {
                next_offset = Some(index + 1);
                next_column = Some(begin);
                break;
            }
            let count = value.chars().count();
            position += count;
            remaining -= value.len();
            rows.push((index + 1, value.to_owned(), begin));
            if count < line_chars {
                next_offset = Some(index + 1);
                next_column = Some(begin + count);
                break;
            }
            if index + 1 < self.line_count() {
                next_offset = Some(index + 2);
                next_column = Some(0);
            } else {
                next_offset = None;
                next_column = None;
            }
        }
        Ok((rows, first, position, next_offset, next_column))
    }

    pub(crate) fn normalized(&self) -> Result<Vec<u8>, CoreError> {
        let mut bytes = reserved(self.text.len() + 1)?;
        let mut copied = 0;
        if self.standard_breaks {
            for end in standard_ends(self.text.as_bytes()) {
                let start = end
                    - if self.text.as_bytes()[end - 1] == b'\n'
                        && self.text.as_bytes().get(end.wrapping_sub(2)) == Some(&b'\r')
                    {
                        2
                    } else {
                        1
                    };
                bytes.extend_from_slice(&self.text.as_bytes()[copied..start]);
                bytes.push(b'\n');
                copied = end;
            }
            bytes.extend_from_slice(&self.text.as_bytes()[copied..]);
            if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
                bytes.push(b'\n');
            }
            return Ok(bytes);
        }
        let mut iterator = self.text.char_indices().peekable();
        while let Some((start, ch)) = iterator.next() {
            if !is_break(ch) {
                continue;
            }
            bytes.extend_from_slice(&self.text.as_bytes()[copied..start]);
            bytes.push(b'\n');
            copied = start + ch.len_utf8();
            if ch == '\r' && iterator.peek().is_some_and(|(_, ch)| *ch == '\n') {
                if let Some((end, _)) = iterator.next() {
                    copied = end + 1;
                }
            }
        }
        bytes.extend_from_slice(&self.text.as_bytes()[copied..]);
        if !bytes.is_empty() && bytes.last() != Some(&b'\n') {
            bytes.push(b'\n');
        }
        Ok(bytes)
    }
}
