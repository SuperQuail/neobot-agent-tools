//! 计算与 Python ensure_ascii JSON 一致的保守字节额度。

use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyString};

fn character_bytes(character: u32) -> usize {
    match character {
        0x08 | 0x09 | 0x0a | 0x0c | 0x0d | 0x22 | 0x5c => 2,
        0x20..=0x7e => 1,
        0..=0xffff => 6,
        _ => 12,
    }
}

pub(super) fn string_bytes(value: &Bound<'_, PyString>) -> PyResult<usize> {
    if let Ok(text) = value.to_str() {
        return Ok(2 + text
            .chars()
            .map(|character| character_bytes(character as u32))
            .sum::<usize>());
    }
    // Python 字符串允许孤立代理项；仅这条稀有路径需要保留码点的临时编码。
    let encoded = value.call_method1("encode", ("utf-32-le", "surrogatepass"))?;
    let bytes = encoded.cast::<PyBytes>()?.as_bytes();
    let mut size = 2;
    for chunk in bytes.chunks_exact(4) {
        let character = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        size += character_bytes(character);
    }
    Ok(size)
}
