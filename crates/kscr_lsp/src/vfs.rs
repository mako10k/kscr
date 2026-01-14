//! Virtual File System for managing document state in LSP
//!
//! This module handles:
//! - Document content tracking (including unsaved changes)
//! - Line/column to byte offset conversion
//! - Document versioning

use std::collections::HashMap;
use tower_lsp::lsp_types::Url;

/// Represents a document in the VFS
#[derive(Debug, Clone)]
pub struct Document {
    /// Document URI
    pub uri: Url,
    /// Document text content
    pub text: String,
    /// Document version (incremented on each change)
    pub version: i32,
    /// Line start positions (byte offsets)
    line_starts: Vec<usize>,
}

impl Document {
    pub fn new(uri: Url, text: String, version: i32) -> Self {
        let line_starts = compute_line_starts(&text);
        Self {
            uri,
            text,
            version,
            line_starts,
        }
    }

    /// Update document content
    pub fn update(&mut self, text: String, version: i32) {
        self.text = text;
        self.version = version;
        self.line_starts = compute_line_starts(&self.text);
    }

    /// Convert LSP position (line, character) to byte offset
    #[allow(dead_code)] // Used in future hover/goto-definition implementations
    pub fn position_to_offset(&self, line: u32, character: u32) -> Option<usize> {
        let line = line as usize;
        if line >= self.line_starts.len() {
            return None;
        }
        
        let line_start = self.line_starts[line];
        let line_end = if line + 1 < self.line_starts.len() {
            self.line_starts[line + 1]
        } else {
            self.text.len()
        };
        
        // Convert UTF-16 code units to UTF-8 byte offset
        let line_text = &self.text[line_start..line_end];
        let mut utf16_offset = 0u32;
        let mut byte_offset = 0usize;
        
        for ch in line_text.chars() {
            if utf16_offset >= character {
                break;
            }
            utf16_offset += ch.len_utf16() as u32;
            byte_offset += ch.len_utf8();
        }
        
        Some(line_start + byte_offset)
    }

    /// Convert byte offset to LSP position (line, character)
    #[allow(dead_code)] // Used in future hover/goto-definition implementations
    pub fn offset_to_position(&self, offset: usize) -> Option<(u32, u32)> {
        if offset > self.text.len() {
            return None;
        }
        
        // Binary search for line
        let line = match self.line_starts.binary_search(&offset) {
            Ok(line) => line,
            Err(line) => line.saturating_sub(1),
        };
        
        if line >= self.line_starts.len() {
            return None;
        }
        
        let line_start = self.line_starts[line];
        let line_text = &self.text[line_start..offset];
        
        // Count UTF-16 code units
        let character = line_text.chars().map(|c| c.len_utf16() as u32).sum();
        
        Some((line as u32, character))
    }
}

/// Compute line start positions (byte offsets)
fn compute_line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    for (i, ch) in text.char_indices() {
        if ch == '\n' {
            starts.push(i + ch.len_utf8());
        }
    }
    starts
}

/// Virtual File System managing all open documents
#[derive(Debug, Default)]
pub struct Vfs {
    documents: HashMap<Url, Document>,
}

impl Vfs {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
        }
    }

    /// Add or update a document
    pub fn insert(&mut self, uri: Url, text: String, version: i32) {
        let doc = Document::new(uri.clone(), text, version);
        self.documents.insert(uri, doc);
    }

    /// Update an existing document
    pub fn update(&mut self, uri: &Url, text: String, version: i32) {
        if let Some(doc) = self.documents.get_mut(uri) {
            doc.update(text, version);
        }
    }

    /// Get a document
    pub fn get(&self, uri: &Url) -> Option<&Document> {
        self.documents.get(uri)
    }

    /// Remove a document
    pub fn remove(&mut self, uri: &Url) {
        self.documents.remove(uri);
    }

    /// Check if a document exists
    #[allow(dead_code)] // Used in future implementations
    pub fn contains(&self, uri: &Url) -> bool {
        self.documents.contains_key(uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_starts() {
        let text = "hello\nworld\nfoo";
        let starts = compute_line_starts(text);
        assert_eq!(starts, vec![0, 6, 12]);
    }

    #[test]
    fn test_position_to_offset() {
        let uri = Url::parse("file:///test.ks").unwrap();
        let doc = Document::new(uri, "hello\nworld\n".to_string(), 1);
        
        assert_eq!(doc.position_to_offset(0, 0), Some(0));
        assert_eq!(doc.position_to_offset(0, 5), Some(5));
        assert_eq!(doc.position_to_offset(1, 0), Some(6));
        assert_eq!(doc.position_to_offset(1, 5), Some(11));
    }

    #[test]
    fn test_offset_to_position() {
        let uri = Url::parse("file:///test.ks").unwrap();
        let doc = Document::new(uri, "hello\nworld\n".to_string(), 1);
        
        assert_eq!(doc.offset_to_position(0), Some((0, 0)));
        assert_eq!(doc.offset_to_position(5), Some((0, 5)));
        assert_eq!(doc.offset_to_position(6), Some((1, 0)));
        assert_eq!(doc.offset_to_position(11), Some((1, 5)));
    }
}
