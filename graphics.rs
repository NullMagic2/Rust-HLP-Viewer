//! WinHelp's block-oriented LZ77 decoder.
//!
//! Flag bits are consumed least-significant bit first. A zero bit emits one literal byte; a one
//! bit consumes a two-byte match token whose low 12 bits encode a backward distance minus one and
//! whose high four bits encode a match length minus three. Matches may overlap, but may never
//! reach before bytes already emitted by the current decompression stream.

use crate::error::HlpError;

/// Decompresses one independent WinHelp LZ77 stream while enforcing the caller's output bound.
pub(crate) fn lz77_decompress(input: &[u8], max_output: usize) -> Result<Vec<u8>, HlpError> {
    let mut input_pos = 0_usize;
    let mut output = Vec::with_capacity(max_output.min(input.len().saturating_mul(2)));

    while input_pos < input.len() && output.len() < max_output {
        let flags = input[input_pos];
        input_pos += 1;

        for bit in 0..8 {
            if output.len() >= max_output {
                break;
            }
            if input_pos >= input.len() {
                break;
            }

            if flags & (1 << bit) == 0 {
                output.push(input[input_pos]);
                input_pos += 1;
                continue;
            }

            if input_pos + 1 >= input.len() {
                return Err(HlpError::UnexpectedEof {
                    context: "WinHelp LZ77 match token",
                });
            }

            let code = u16::from_le_bytes([input[input_pos], input[input_pos + 1]]);
            input_pos += 2;

            let distance = usize::from(code & 0x0FFF) + 1;
            let length = usize::from(code >> 12) + 3;
            if distance > output.len() {
                return Err(HlpError::invalid(
                    "WinHelp LZ77",
                    format!(
                        "backreference distance {distance} exceeds {} decoded bytes",
                        output.len()
                    ),
                ));
            }

            for _ in 0..length {
                if output.len() >= max_output {
                    break;
                }
                // Recompute against the growing output so overlapping matches repeat correctly.
                let source = output.len() - distance;
                let value = output[source];
                output.push(value);
            }
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_literal_group() {
        let input = [0x00, b'W', b'i', b'n', b'H', b'e', b'l', b'p', b'!'];
        assert_eq!(lz77_decompress(&input, 64).unwrap(), b"WinHelp!");
    }

    #[test]
    fn rejects_backreference_before_decoded_output() {
        let input = [0x01, 0x00, 0x00];
        assert!(lz77_decompress(&input, 64).is_err());
    }

    #[test]
    fn overlapping_backreference_reuses_new_output() {
        // Four literals followed by a distance-4, length-4 match: "ABCDABCD".
        let input = [0x10, b'A', b'B', b'C', b'D', 0x03, 0x10];
        assert_eq!(lz77_decompress(&input, 64).unwrap(), b"ABCDABCD");
    }

    #[test]
    fn truncated_match_is_rejected() {
        let input = [0x02, b'X', 0x00];
        assert!(lz77_decompress(&input, 64).is_err());
    }

    #[test]
    fn output_cap_is_respected() {
        let input = [0x00, b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H'];
        assert_eq!(lz77_decompress(&input, 3).unwrap(), b"ABC");
    }
}
