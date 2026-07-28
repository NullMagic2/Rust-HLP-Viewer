//! Small endian-aware cursor used by every binary HLP parser.

use crate::HlpError;

/// A bounds-checked little-endian reader over a borrowed byte slice.
pub(crate) struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    context: &'static str,
}

impl<'a> Reader<'a> {
    /// Creates a reader starting at byte zero.
    pub(crate) const fn new(bytes: &'a [u8], context: &'static str) -> Self {
        Self {
            bytes,
            position: 0,
            context,
        }
    }

    /// Returns the current cursor offset.
    pub(crate) const fn position(&self) -> usize {
        self.position
    }

    /// Returns how many bytes remain unread.
    pub(crate) const fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    /// Reads an unsigned byte.
    pub(crate) fn read_u8(&mut self) -> Result<u8, HlpError> {
        let value = *self
            .bytes
            .get(self.position)
            .ok_or(HlpError::UnexpectedEof {
                context: self.context,
            })?;
        self.position += 1;
        Ok(value)
    }

    /// Reads an unsigned 16-bit little-endian integer.
    pub(crate) fn read_u16(&mut self) -> Result<u16, HlpError> {
        let bytes = self.read_array::<2>()?;
        Ok(u16::from_le_bytes(bytes))
    }

    /// Reads a signed 16-bit little-endian integer.
    pub(crate) fn read_i16(&mut self) -> Result<i16, HlpError> {
        let bytes = self.read_array::<2>()?;
        Ok(i16::from_le_bytes(bytes))
    }

    /// Reads an unsigned 32-bit little-endian integer.
    pub(crate) fn read_u32(&mut self) -> Result<u32, HlpError> {
        let bytes = self.read_array::<4>()?;
        Ok(u32::from_le_bytes(bytes))
    }

    /// Reads a signed 32-bit little-endian integer.
    pub(crate) fn read_i32(&mut self) -> Result<i32, HlpError> {
        let bytes = self.read_array::<4>()?;
        Ok(i32::from_le_bytes(bytes))
    }

    /// Reads an exact number of bytes and advances the cursor.
    pub(crate) fn read_bytes(&mut self, count: usize) -> Result<&'a [u8], HlpError> {
        let end = self
            .position
            .checked_add(count)
            .ok_or_else(|| HlpError::invalid(self.context, "byte range overflow"))?;
        let result = self
            .bytes
            .get(self.position..end)
            .ok_or(HlpError::UnexpectedEof {
                context: self.context,
            })?;
        self.position = end;
        Ok(result)
    }

    /// Reads a NUL-terminated byte string within this reader's boundary.
    pub(crate) fn read_c_string(&mut self) -> Result<&'a [u8], HlpError> {
        let rest = self
            .bytes
            .get(self.position..)
            .ok_or(HlpError::UnexpectedEof {
                context: self.context,
            })?;
        let relative_end = rest
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| HlpError::invalid(self.context, "unterminated string"))?;
        let start = self.position;
        let end = start + relative_end;
        self.position = end + 1;
        Ok(&self.bytes[start..end])
    }

    /// Reads a fixed-size byte array.
    fn read_array<const N: usize>(&mut self) -> Result<[u8; N], HlpError> {
        let bytes = self.read_bytes(N)?;
        let mut array = [0_u8; N];
        array.copy_from_slice(bytes);
        Ok(array)
    }
}
