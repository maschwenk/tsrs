use crate::TextRange;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextChange {
    pub text_range: TextRange,
    pub new_text: String,
}

impl TextChange {
    pub fn pos(&self) -> i32 {
        self.text_range.pos()
    }

    pub fn end(&self) -> i32 {
        self.text_range.end()
    }

    pub fn apply_to(&self, text: &str) -> String {
        format!("{}{}{}", &text[..self.pos() as usize], self.new_text, &text[self.end() as usize..])
    }
}

pub fn apply_bulk_edits(text: &str, edits: &[TextChange]) -> String {
    let mut b = String::with_capacity(text.len());
    let mut last_end = 0usize;
    for e in edits {
        let start = e.text_range.pos() as usize;
        if start != last_end {
            b.push_str(&text[last_end..start]);
        }
        b.push_str(&e.new_text);

        last_end = e.text_range.end() as usize;
    }
    b.push_str(&text[last_end..]);

    b
}
