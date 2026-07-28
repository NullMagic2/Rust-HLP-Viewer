//! Parser for the internal B+ tree that forms an HLP file's directory.

use std::collections::HashSet;

use crate::container::DirectoryEntry;
use crate::reader::Reader;
use crate::HlpError;

const DIRECTORY_BTREE_MAGIC: u16 = 0x293B;
const BTREE_HEADER_SIZE: usize = 38;

/// Header common to the HLP directory B+ tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BTreeHeader {
    pub(crate) page_size: u16,
    pub(crate) root_page: i16,
    pub(crate) total_pages: i16,
    pub(crate) levels: i16,
    pub(crate) total_entries: u32,
}

/// Parses the directory B+ tree and returns its leaf records in logical order.
pub(crate) fn parse_directory(content: &[u8]) -> Result<Vec<DirectoryEntry>, HlpError> {
    let header = parse_header(content)?;
    validate_storage(content, &header)?;

    let mut page_number = header.root_page;
    for _ in 1..header.levels {
        let page = page(content, &header, page_number)?;
        let mut reader = Reader::new(page, "directory index page");
        let _unused = reader.read_u16()?;
        let entries = reader.read_i16()?;
        let previous_page = reader.read_i16()?;
        if entries < 0 {
            return Err(HlpError::invalid(
                "directory index page",
                "negative entry count",
            ));
        }
        if previous_page < 0 {
            return Err(HlpError::invalid(
                "directory index page",
                "leftmost child page is negative",
            ));
        }
        page_number = previous_page;
    }

    let mut result = Vec::with_capacity(header.total_entries as usize);
    let mut visited = HashSet::new();

    loop {
        if !visited.insert(page_number) {
            return Err(HlpError::invalid(
                "directory leaf chain",
                format!("cycle detected at page {page_number}"),
            ));
        }

        let page_bytes = page(content, &header, page_number)?;
        let (mut entries, next_page) = parse_leaf(page_bytes)?;
        result.append(&mut entries);

        if result.len() > header.total_entries as usize {
            return Err(HlpError::invalid(
                "directory B+ tree",
                "leaf records exceed header entry count",
            ));
        }

        if next_page < 0 {
            break;
        }
        page_number = next_page;
    }

    if result.len() != header.total_entries as usize {
        return Err(HlpError::invalid(
            "directory B+ tree",
            format!(
                "header declares {} entries but leaf chain contains {}",
                header.total_entries,
                result.len()
            ),
        ));
    }

    Ok(result)
}

/// Parses and validates the fixed directory B+ tree header.
fn parse_header(content: &[u8]) -> Result<BTreeHeader, HlpError> {
    let mut reader = Reader::new(content, "directory B+ tree header");
    let magic = reader.read_u16()?;
    if magic != DIRECTORY_BTREE_MAGIC {
        return Err(HlpError::InvalidMagic {
            context: "directory B+ tree",
            expected: u32::from(DIRECTORY_BTREE_MAGIC),
            actual: u32::from(magic),
        });
    }

    let _flags = reader.read_u16()?;
    let page_size = reader.read_u16()?;
    let _structure = reader.read_bytes(16)?;
    let must_be_zero = reader.read_i16()?;
    let _page_splits = reader.read_i16()?;
    let root_page = reader.read_i16()?;
    let must_be_neg_one = reader.read_i16()?;
    let total_pages = reader.read_i16()?;
    let levels = reader.read_i16()?;
    let total_entries = reader.read_u32()?;

    if must_be_zero != 0 {
        return Err(HlpError::invalid(
            "directory B+ tree header",
            format!("reserved zero field contains {must_be_zero}"),
        ));
    }
    if must_be_neg_one != -1 {
        return Err(HlpError::invalid(
            "directory B+ tree header",
            format!("reserved -1 field contains {must_be_neg_one}"),
        ));
    }
    if page_size < 8 {
        return Err(HlpError::invalid(
            "directory B+ tree header",
            format!("page size {page_size} is too small"),
        ));
    }
    if total_pages <= 0 || levels <= 0 {
        return Err(HlpError::invalid(
            "directory B+ tree header",
            "tree must contain at least one page and one level",
        ));
    }
    if root_page < 0 || root_page >= total_pages {
        return Err(HlpError::invalid(
            "directory B+ tree header",
            format!("root page {root_page} is outside 0..{total_pages}"),
        ));
    }

    Ok(BTreeHeader {
        page_size,
        root_page,
        total_pages,
        levels,
        total_entries,
    })
}

/// Ensures all fixed-size B+ tree pages fit inside the internal directory stream.
fn validate_storage(content: &[u8], header: &BTreeHeader) -> Result<(), HlpError> {
    let page_bytes = usize::try_from(header.total_pages)
        .map_err(|_| HlpError::invalid("directory B+ tree", "negative page count"))?
        .checked_mul(usize::from(header.page_size))
        .ok_or_else(|| HlpError::invalid("directory B+ tree", "page storage overflow"))?;
    let required = BTREE_HEADER_SIZE
        .checked_add(page_bytes)
        .ok_or_else(|| HlpError::invalid("directory B+ tree", "tree size overflow"))?;
    if required > content.len() {
        return Err(HlpError::invalid(
            "directory B+ tree",
            format!("tree needs {required} bytes but stream has {}", content.len()),
        ));
    }
    Ok(())
}

/// Returns one fixed-size page after checking its number and byte range.
fn page<'a>(
    content: &'a [u8],
    header: &BTreeHeader,
    page_number: i16,
) -> Result<&'a [u8], HlpError> {
    if page_number < 0 || page_number >= header.total_pages {
        return Err(HlpError::invalid(
            "directory B+ tree",
            format!("page {page_number} is outside the tree"),
        ));
    }

    let page_number = usize::try_from(page_number)
        .map_err(|_| HlpError::invalid("directory B+ tree", "negative page number"))?;
    let page_size = usize::from(header.page_size);
    let start = BTREE_HEADER_SIZE
        .checked_add(page_number.checked_mul(page_size).ok_or_else(|| {
            HlpError::invalid("directory B+ tree", "page offset overflow")
        })?)
        .ok_or_else(|| HlpError::invalid("directory B+ tree", "page offset overflow"))?;
    let end = start
        .checked_add(page_size)
        .ok_or_else(|| HlpError::invalid("directory B+ tree", "page end overflow"))?;
    content
        .get(start..end)
        .ok_or(HlpError::UnexpectedEof {
            context: "directory B+ tree page",
        })
}

/// Parses one directory leaf page and returns records plus the linked next-page number.
fn parse_leaf(page: &[u8]) -> Result<(Vec<DirectoryEntry>, i16), HlpError> {
    let mut header_reader = Reader::new(page, "directory leaf page");
    let unused = usize::from(header_reader.read_u16()?);
    let entry_count = header_reader.read_i16()?;
    let _previous_page = header_reader.read_i16()?;
    let next_page = header_reader.read_i16()?;

    if entry_count < 0 {
        return Err(HlpError::invalid(
            "directory leaf page",
            "negative entry count",
        ));
    }
    if unused > page.len().saturating_sub(8) {
        return Err(HlpError::invalid(
            "directory leaf page",
            format!("unused-byte count {unused} exceeds page capacity"),
        ));
    }

    let logical_end = page.len() - unused;
    let records = page
        .get(8..logical_end)
        .ok_or(HlpError::UnexpectedEof {
            context: "directory leaf records",
        })?;
    let mut reader = Reader::new(records, "directory leaf records");
    let mut entries = Vec::with_capacity(entry_count as usize);

    for _ in 0..entry_count {
        let raw_name = reader.read_c_string()?;
        if raw_name.is_empty() {
            return Err(HlpError::invalid(
                "directory leaf record",
                "empty internal filename",
            ));
        }
        let file_offset = reader.read_u32()?;
        entries.push(DirectoryEntry {
            name: decode_directory_name(raw_name),
            file_offset,
        });
    }

    Ok((entries, next_page))
}

/// Converts the mostly-ASCII internal directory filename to a Rust string without panicking.
fn decode_directory_name(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Returns all linked leaf pages of an HLP B+ tree in logical order.
///
/// This is shared by optional indexes such as `|KWBTREE`, whose leaf record
/// payload differs from the directory but whose fixed header/page linkage is
/// identical.
pub(crate) fn leaf_pages(content: &[u8]) -> Result<Vec<&[u8]>, HlpError> {
    let header = parse_header(content)?;
    validate_storage(content, &header)?;

    let mut page_number = header.root_page;
    for _ in 1..header.levels {
        let page_bytes = page(content, &header, page_number)?;
        let mut reader = Reader::new(page_bytes, "B+ tree index page");
        let _unused = reader.read_u16()?;
        let entries = reader.read_i16()?;
        let leftmost_child = reader.read_i16()?;
        if entries < 0 {
            return Err(HlpError::invalid("B+ tree index page", "negative entry count"));
        }
        if leftmost_child < 0 {
            return Err(HlpError::invalid(
                "B+ tree index page",
                "leftmost child page is negative",
            ));
        }
        page_number = leftmost_child;
    }

    let mut pages = Vec::new();
    let mut visited = HashSet::new();
    loop {
        if !visited.insert(page_number) {
            return Err(HlpError::invalid(
                "B+ tree leaf chain",
                format!("cycle detected at page {page_number}"),
            ));
        }
        let page_bytes = page(content, &header, page_number)?;
        let mut reader = Reader::new(page_bytes, "B+ tree leaf page");
        let unused = usize::from(reader.read_u16()?);
        let entry_count = reader.read_i16()?;
        let _previous_page = reader.read_i16()?;
        let next_page = reader.read_i16()?;
        if entry_count < 0 {
            return Err(HlpError::invalid("B+ tree leaf page", "negative entry count"));
        }
        if unused > page_bytes.len().saturating_sub(8) {
            return Err(HlpError::invalid(
                "B+ tree leaf page",
                format!("unused-byte count {unused} exceeds page capacity"),
            ));
        }
        pages.push(page_bytes);
        if next_page < 0 {
            break;
        }
        page_number = next_page;
    }
    Ok(pages)
}
