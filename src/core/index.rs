//! 保存行起点和稀疏 Unicode 检查点，避免每次翻页重新扫描完整文本。

use super::lines::{floor_char_boundary, has_standard_breaks, scan_lines};
use super::{CoreError, reserved};

const CHECKPOINT_BYTES: usize = 256;

/// 输入已被限制在 16 MiB 内，字节和字符偏移都可安全存为 u32。
#[derive(Clone, Copy)]
pub(super) struct Position {
    pub(super) byte_offset: u32,
    pub(super) character_offset: u32,
}

pub(super) struct LineIndex {
    positions: Vec<Position>,
    checkpoints: Vec<Position>,
    characters: usize,
    ascii: bool,
    pub(super) standard_breaks: bool,
}

impl LineIndex {
    pub(super) fn build(text: &str) -> Result<Self, CoreError> {
        let standard_breaks = has_standard_breaks(text.as_bytes());
        let (positions, characters) = build_line_positions(text, standard_breaks)?;
        let ascii = text.is_ascii();
        let checkpoints = if ascii {
            Vec::new()
        } else {
            build_checkpoints(text)?
        };
        Ok(Self {
            positions,
            checkpoints,
            characters,
            ascii,
            standard_breaks,
        })
    }

    pub(super) fn characters(&self) -> usize {
        self.characters
    }

    pub(super) fn line_count(&self) -> usize {
        self.positions.len() - 1
    }

    pub(super) fn allocated_bytes(&self) -> usize {
        self.positions.capacity() * std::mem::size_of::<Position>()
            + self.checkpoints.capacity() * std::mem::size_of::<Position>()
    }

    pub(super) fn start(&self, index: usize) -> usize {
        self.positions
            .get(index)
            .map_or(self.characters, |position| {
                position.character_offset as usize
            })
    }

    pub(super) fn line_bounds(&self, index: usize) -> Result<(Position, Position), CoreError> {
        let next = index.checked_add(1).ok_or(CoreError::LineIndex)?;
        match (self.positions.get(index), self.positions.get(next)) {
            (Some(start), Some(end)) => Ok((*start, *end)),
            _ => Err(CoreError::LineIndex),
        }
    }

    pub(super) fn byte_for_character(
        &self,
        text: &str,
        character: usize,
    ) -> Result<usize, CoreError> {
        if character > self.characters {
            return Err(CoreError::Argument("column exceeds line length"));
        }
        if self.ascii {
            return Ok(character);
        }
        if character == self.characters {
            return Ok(text.len());
        }
        let checkpoint_index = self
            .checkpoints
            .partition_point(|position| position.character_offset as usize <= character);
        let checkpoint = self
            .checkpoints
            .get(checkpoint_index.saturating_sub(1))
            .ok_or(CoreError::LineIndex)?;
        let start_byte = checkpoint.byte_offset as usize;
        let suffix = text.get(start_byte..).ok_or(CoreError::LineIndex)?;
        suffix
            .char_indices()
            .nth(character - checkpoint.character_offset as usize)
            .map(|(offset, _)| start_byte + offset)
            .ok_or(CoreError::LineIndex)
    }
}

fn build_line_positions(text: &str, standard: bool) -> Result<(Vec<Position>, usize), CoreError> {
    let mut boundary_count = 0;
    let mut last_byte = 0;
    let characters = scan_lines(text, standard, |byte, _| {
        boundary_count += 1;
        last_byte = byte;
    });
    let has_tail = last_byte < text.len();
    // 精确预留容量，防止密集换行触发扩容时旧索引与新索引同时占用大量内存。
    let mut positions = reserved(boundary_count + usize::from(has_tail) + 1)?;
    positions.push(Position {
        byte_offset: 0,
        character_offset: 0,
    });
    scan_lines(text, standard, |byte, character| {
        positions.push(Position {
            byte_offset: byte as u32,
            character_offset: character as u32,
        });
    });
    if has_tail {
        positions.push(Position {
            byte_offset: text.len() as u32,
            character_offset: characters as u32,
        });
    }
    Ok((positions, characters))
}

fn build_checkpoints(text: &str) -> Result<Vec<Position>, CoreError> {
    // 每块最多为对齐 UTF-8 边界回退三个字节，按最小步长预留容量，避免二次分配。
    let mut checkpoints = reserved(text.len().div_ceil(CHECKPOINT_BYTES - 3))?;
    let mut byte_offset = 0;
    let mut character_offset = 0;
    while byte_offset < text.len() {
        checkpoints.push(Position {
            byte_offset: byte_offset as u32,
            character_offset: character_offset as u32,
        });
        let end_byte = floor_char_boundary(text, (byte_offset + CHECKPOINT_BYTES).min(text.len()));
        character_offset += text[byte_offset..end_byte].chars().count();
        byte_offset = end_byte;
    }
    Ok(checkpoints)
}
