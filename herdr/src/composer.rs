//! A small multi-line text input: the chat composer and the sidebar's inline fields.

use unicode_width::UnicodeWidthChar;

#[derive(Default, Clone)]
pub struct Composer {
    pub text: String,
    /// Byte offset, always on a char boundary.
    pub cursor: usize,
}

impl Composer {
    pub fn with(text: &str) -> Self {
        Self { text: text.to_string(), cursor: text.len() }
    }
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
    pub fn set(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.text.len();
    }
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.text)
    }
    pub fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }
    pub fn insert_str(&mut self, s: &str) {
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }
    fn prev(&self) -> Option<usize> {
        self.text[..self.cursor].char_indices().next_back().map(|(i, _)| i)
    }
    fn next(&self) -> Option<usize> {
        self.text[self.cursor..].chars().next().map(|c| self.cursor + c.len_utf8())
    }
    pub fn backspace(&mut self) {
        if let Some(p) = self.prev() {
            self.text.replace_range(p..self.cursor, "");
            self.cursor = p;
        }
    }
    pub fn delete(&mut self) {
        if let Some(n) = self.next() {
            self.text.replace_range(self.cursor..n, "");
        }
    }
    pub fn left(&mut self) {
        if let Some(p) = self.prev() {
            self.cursor = p;
        }
    }
    pub fn right(&mut self) {
        if let Some(n) = self.next() {
            self.cursor = n;
        }
    }
    pub fn home(&mut self) {
        self.cursor = self.text[..self.cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
    }
    pub fn end(&mut self) {
        self.cursor = self.text[self.cursor..].find('\n').map(|i| self.cursor + i).unwrap_or(self.text.len());
    }
    /// Deletes the word before the cursor (ctrl+w).
    pub fn delete_word(&mut self) {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        let start = trimmed.rfind(char::is_whitespace).map(|i| i + 1).unwrap_or(0);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }
    /// Deletes back to the start of the line (ctrl+u).
    pub fn delete_line(&mut self) {
        let start = self.text[..self.cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
        self.text.replace_range(start..self.cursor, "");
        self.cursor = start;
    }
    /// Moves to the same column on the line above or below; false when there is none.
    pub fn vertical(&mut self, down: bool) -> bool {
        let line_start = self.text[..self.cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = self.text[line_start..self.cursor].chars().count();
        let target_start = if down {
            match self.text[self.cursor..].find('\n') {
                Some(i) => self.cursor + i + 1,
                None => return false,
            }
        } else {
            if line_start == 0 {
                return false;
            }
            self.text[..line_start - 1].rfind('\n').map(|i| i + 1).unwrap_or(0)
        };
        let line_end = self.text[target_start..].find('\n').map(|i| target_start + i).unwrap_or(self.text.len());
        let mut pos = target_start;
        for (n, (i, c)) in self.text[target_start..line_end].char_indices().enumerate() {
            if n == col {
                break;
            }
            pos = target_start + i + c.len_utf8();
        }
        self.cursor = pos;
        true
    }

    /// The text broken into rows of `w` columns (by character, so the cursor maps exactly),
    /// and the cursor's row and column.
    pub fn layout(&self, w: usize) -> (Vec<String>, (usize, usize)) {
        let w = w.max(2);
        let mut rows = vec![String::new()];
        let mut col = 0;
        let mut cur = (0, 0);
        for (i, c) in self.text.char_indices() {
            if i == self.cursor {
                cur = (rows.len() - 1, col);
            }
            if c == '\n' {
                rows.push(String::new());
                col = 0;
                continue;
            }
            let cw = c.width().unwrap_or(0);
            if col + cw > w {
                rows.push(String::new());
                col = 0;
                if i == self.cursor {
                    cur = (rows.len() - 1, 0);
                }
            }
            rows.last_mut().unwrap().push(c);
            col += cw;
        }
        if self.cursor >= self.text.len() {
            if col >= w {
                rows.push(String::new());
                col = 0;
            }
            cur = (rows.len() - 1, col);
        }
        (rows, cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edits_around_the_cursor() {
        let mut c = Composer::default();
        for ch in "héllo".chars() {
            c.insert(ch);
        }
        c.left();
        c.backspace();
        assert_eq!(c.text, "hélo");
        c.home();
        c.delete();
        assert_eq!(c.text, "élo");
        c.end();
        c.insert_str(" world");
        c.delete_word();
        assert_eq!(c.text, "élo ");
    }

    #[test]
    fn lays_out_rows_with_the_cursor() {
        let c = Composer::with("abcdef\ngh");
        let (rows, cur) = c.layout(4);
        assert_eq!(rows, vec!["abcd", "ef", "gh"]);
        assert_eq!(cur, (2, 2));
        let mut c = Composer::with("abcd");
        assert_eq!(c.layout(4).1, (1, 0), "a full row puts the cursor on the next");
        c.home();
        assert_eq!(c.layout(4).1, (0, 0));
    }

    #[test]
    fn moves_between_lines() {
        let mut c = Composer::with("one\nlonger line");
        assert!(c.vertical(false));
        assert_eq!(c.cursor, 3);
        assert!(!c.vertical(false));
        assert!(c.vertical(true));
        assert_eq!(&c.text[c.cursor..], "ger line");
    }
}
