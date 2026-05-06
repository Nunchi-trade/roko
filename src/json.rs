use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub message: String,
    pub index: usize,
}

impl JsonError {
    fn new(index: usize, message: impl Into<String>) -> Self {
        Self { index, message: message.into() }
    }
}

pub fn parse_json(input: &str) -> Result<JsonValue, JsonError> {
    let mut parser = Parser { input, pos: 0 };
    let value = parser.parse_value()?;
    parser.skip_ws();
    if parser.pos != input.len() {
        return Err(JsonError::new(parser.pos, "trailing characters"));
    }
    Ok(value)
}

struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn parse_value(&mut self) -> Result<JsonValue, JsonError> {
        self.skip_ws();
        match self.peek_char() {
            Some('n') => { self.expect_literal("null")?; Ok(JsonValue::Null) }
            Some('t') => { self.expect_literal("true")?; Ok(JsonValue::Bool(true)) }
            Some('f') => { self.expect_literal("false")?; Ok(JsonValue::Bool(false)) }
            Some('"') => Ok(JsonValue::String(self.parse_string()?)),
            Some('[') => self.parse_array(),
            Some('{') => self.parse_object(),
            Some('-') | Some('0'..='9') => self.parse_number().map(JsonValue::Number),
            Some(_) => Err(JsonError::new(self.pos, "unexpected character")),
            None => Err(JsonError::new(self.pos, "unexpected end of input")),
        }
    }

    fn parse_array(&mut self) -> Result<JsonValue, JsonError> {
        self.consume_char();
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some(']') { self.consume_char(); break; }
            items.push(self.parse_value()?);
            self.skip_ws();
            match self.peek_char() {
                Some(',') => { self.consume_char(); }
                Some(']') => { self.consume_char(); break; }
                _ => return Err(JsonError::new(self.pos, "expected ',' or ']'")),
            }
        }
        Ok(JsonValue::Array(items))
    }

    fn parse_object(&mut self) -> Result<JsonValue, JsonError> {
        self.consume_char();
        let mut map = BTreeMap::new();
        loop {
            self.skip_ws();
            if self.peek_char() == Some('}') { self.consume_char(); break; }
            let key = self.parse_string()?;
            self.skip_ws();
            if self.consume_char() != Some(':') { return Err(JsonError::new(self.pos, "expected ':'")); }
            let value = self.parse_value()?;
            map.insert(key, value);
            self.skip_ws();
            match self.peek_char() {
                Some(',') => { self.consume_char(); }
                Some('}') => { self.consume_char(); break; }
                _ => return Err(JsonError::new(self.pos, "expected ',' or '}'")),
            }
        }
        Ok(JsonValue::Object(map))
    }

    fn parse_string(&mut self) -> Result<String, JsonError> {
        if self.consume_char() != Some('"') { return Err(JsonError::new(self.pos, "expected string")); }
        let mut out = String::new();
        while let Some(ch) = self.consume_char() {
            match ch {
                '"' => return Ok(out),
                '\\' => {
                    let esc = self.consume_char().ok_or_else(|| JsonError::new(self.pos, "unterminated escape"))?;
                    match esc {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{0008}'),
                        'f' => out.push('\u{000C}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let cp = self.parse_hex4()?;
                            if let Some(c) = char::from_u32(cp) { out.push(c); } else { return Err(JsonError::new(self.pos, "invalid unicode escape")); }
                        }
                        _ => return Err(JsonError::new(self.pos, "invalid escape")),
                    }
                }
                c if c.is_control() => return Err(JsonError::new(self.pos, "control character in string")),
                _ => out.push(ch),
            }
        }
        Err(JsonError::new(self.pos, "unterminated string"))
    }

    fn parse_number(&mut self) -> Result<f64, JsonError> {
        let start = self.pos;
        if self.peek_char() == Some('-') { self.consume_char(); }
        match self.peek_char() {
            Some('0') => { self.consume_char(); }
            Some('1'..='9') => { self.consume_while(|c| c.is_ascii_digit()); }
            _ => return Err(JsonError::new(self.pos, "invalid number")),
        }
        if self.peek_char() == Some('.') {
            self.consume_char();
            if !self.peek_char().map(|c| c.is_ascii_digit()).unwrap_or(false) { return Err(JsonError::new(self.pos, "invalid number")); }
            self.consume_while(|c| c.is_ascii_digit());
        }
        if matches!(self.peek_char(), Some('e' | 'E')) {
            self.consume_char();
            if matches!(self.peek_char(), Some('+' | '-')) { self.consume_char(); }
            if !self.peek_char().map(|c| c.is_ascii_digit()).unwrap_or(false) { return Err(JsonError::new(self.pos, "invalid number")); }
            self.consume_while(|c| c.is_ascii_digit());
        }
        self.input[start..self.pos].parse::<f64>().map_err(|_| JsonError::new(start, "invalid number"))
    }

    fn parse_hex4(&mut self) -> Result<u32, JsonError> {
        let mut value = 0u32;
        for _ in 0..4 {
            let c = self.consume_char().ok_or_else(|| JsonError::new(self.pos, "unterminated unicode escape"))?;
            value = (value << 4) | c.to_digit(16).ok_or_else(|| JsonError::new(self.pos, "invalid unicode escape"))?;
        }
        Ok(value)
    }

    fn expect_literal(&mut self, lit: &str) -> Result<(), JsonError> {
        if self.input[self.pos..].starts_with(lit) { self.pos += lit.len(); Ok(()) } else { Err(JsonError::new(self.pos, "invalid literal")) }
    }

    fn skip_ws(&mut self) { self.consume_while(|c| c.is_whitespace()); }
    fn peek_char(&self) -> Option<char> { self.input[self.pos..].chars().next() }
    fn consume_char(&mut self) -> Option<char> { let ch = self.peek_char()?; self.pos += ch.len_utf8(); Some(ch) }
    fn consume_while<F: Fn(char) -> bool>(&mut self, f: F) { while self.peek_char().map_or(false, &f) { self.consume_char(); } }
}
