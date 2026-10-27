//! 提供不可变 UTF-8 快照与有界分页；行扫描和索引分别由独立模块维护。

mod index;
mod lines;

use std::sync::Arc;

use index::LineIndex;

pub(crate) const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_TEXT_BYTES: usize = 65536;

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

pub(crate) struct SnapshotData {
    text: Box<str>,
    index: LineIndex,
}

// 行三元组仅沿用 Python 接口的约定，依次为行号、文本和起始列。
pub(crate) type Row = (usize, String, usize);

pub(crate) struct Page {
    pub(crate) rows: Vec<Row>,
    pub(crate) start_character: usize,
    pub(crate) end_character: usize,
    pub(crate) next_offset: Option<usize>,
    pub(crate) next_column: Option<usize>,
}

struct FragmentBounds {
    start_byte: usize,
    end_byte: usize,
    remaining_characters: usize,
}

fn reserved<T>(count: usize) -> Result<Vec<T>, CoreError> {
    let mut values = Vec::new();
    values
        .try_reserve_exact(count)
        .map_err(|_| CoreError::Allocation)?;
    Ok(values)
}

impl SnapshotData {
    pub(crate) fn create(data: &[u8]) -> Result<Arc<Self>, CoreError> {
        if data.len() > MAX_FILE_BYTES {
            return Err(CoreError::TooLarge);
        }
        let text = std::str::from_utf8(data)?;
        let index = LineIndex::build(text)?;
        let mut owned = reserved(data.len())?;
        owned.extend_from_slice(data);
        let text = String::from_utf8(owned)
            .map_err(|error| CoreError::Utf8(error.utf8_error()))?
            .into_boxed_str();
        Ok(Arc::new(Self { text, index }))
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn size(&self) -> usize {
        self.text.len()
    }

    pub(crate) fn characters(&self) -> usize {
        self.index.characters()
    }

    pub(crate) fn line_count(&self) -> usize {
        self.index.line_count()
    }

    pub(crate) fn memory_size(&self) -> usize {
        std::mem::size_of::<Self>() + self.text.len() + self.index.allocated_bytes()
    }

    pub(crate) fn start(&self, index: usize) -> usize {
        self.index.start(index)
    }

    pub(crate) fn line(&self, index: usize, keepends: bool) -> Result<&str, CoreError> {
        let (start, end) = self.index.line_bounds(index)?;
        let line = self
            .text
            .get(start.byte_offset as usize..end.byte_offset as usize)
            .ok_or(CoreError::LineIndex)?;
        if keepends {
            return Ok(line);
        }
        if let Some(without_crlf) = line.strip_suffix("\r\n") {
            return Ok(without_crlf);
        }
        let Some(separator) = line
            .chars()
            .next_back()
            .filter(|character| lines::is_break(*character))
        else {
            return Ok(line);
        };
        Ok(&line[..line.len() - separator.len_utf8()])
    }

    fn fragment_bounds(
        &self,
        index: usize,
        column: usize,
        limit: usize,
    ) -> Result<FragmentBounds, CoreError> {
        let (start, end) = self.index.line_bounds(index)?;
        let line_characters = (end.character_offset - start.character_offset) as usize;
        if column > line_characters {
            return Err(CoreError::Argument("column exceeds line length"));
        }
        let character_offset = start.character_offset as usize + column;
        let remaining_characters = line_characters - column;
        let start_byte = if column == 0 {
            start.byte_offset as usize
        } else {
            self.index
                .byte_for_character(&self.text, character_offset)?
        };
        let end_byte = if limit >= remaining_characters {
            end.byte_offset as usize
        } else {
            self.index
                .byte_for_character(&self.text, character_offset + limit)?
        };
        Ok(FragmentBounds {
            start_byte,
            end_byte,
            remaining_characters,
        })
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
        let bounds = self.fragment_bounds(index, column, limit)?;
        Ok((
            self.text[bounds.start_byte..bounds.end_byte].to_owned(),
            bounds.remaining_characters,
        ))
    }

    fn validate_page(
        &self,
        offset: usize,
        limit: usize,
        column: usize,
        budget: usize,
    ) -> Result<(), CoreError> {
        let invalid_offset = offset == 0 || offset > MAX_FILE_BYTES + 1;
        let invalid_limit = limit == 0 || limit > 2000;
        if invalid_offset || invalid_limit || column > MAX_FILE_BYTES || budget > MAX_TEXT_BYTES {
            return Err(CoreError::Argument(
                "Invalid read offset, limit, column or byte budget",
            ));
        }
        if column == 0 {
            return Ok(());
        }
        if offset > self.line_count() || column > self.start(offset) - self.start(offset - 1) {
            return Err(CoreError::Argument("column exceeds line length"));
        }
        Ok(())
    }

    pub(crate) fn page(
        &self,
        offset: usize,
        limit: usize,
        column: usize,
        budget: usize,
    ) -> Result<Page, CoreError> {
        self.validate_page(offset, limit, column, budget)?;
        let start_character = self.start(offset - 1) + column;
        let mut end_character = start_character;
        let mut remaining_bytes = budget;
        let mut rows = Vec::new();
        // 续读行列成对维护，避免其中一项有值、另一项为空。
        let mut continuation = None;
        for index in offset - 1..self.line_count().min(offset - 1 + limit) {
            let begin_column = if index == offset - 1 { column } else { 0 };
            let bounds = self.fragment_bounds(index, begin_column, remaining_bytes)?;
            let end_byte = lines::floor_char_boundary(
                &self.text,
                bounds.end_byte.min(bounds.start_byte + remaining_bytes),
            );
            let value = &self.text[bounds.start_byte..end_byte];
            if value.is_empty() && bounds.remaining_characters > 0 {
                continuation = Some((index + 1, begin_column));
                break;
            }
            let characters = value.chars().count();
            end_character += characters;
            remaining_bytes -= value.len();
            rows.push((index + 1, value.to_owned(), begin_column));
            if characters < bounds.remaining_characters {
                continuation = Some((index + 1, begin_column + characters));
                break;
            }
            continuation = (index + 1 < self.line_count()).then_some((index + 2, 0));
        }
        Ok(Page {
            rows,
            start_character,
            end_character,
            next_offset: continuation.map(|(offset, _)| offset),
            next_column: continuation.map(|(_, column)| column),
        })
    }

    pub(crate) fn normalized(&self) -> Result<Vec<u8>, CoreError> {
        lines::normalized(&self.text, self.index.standard_breaks)
    }
}
