// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 The Gleam contributors

// A simple reader for Erlang terms in the `file:consult/1` format.
//
// `file:consult/1` accepts atoms, characters, floats, integers, strings,
// binaries, lists, tuples, maps, `fun Module:Name/Arity`, and a unary `+` or
// `-` applied to a number. The syntax for each is described in the Erlang
// Reference Manual (https://www.erlang.org/doc/system/data_types.html).
//
// This reader does not support:
// - characters such as `$c`
// - floats
// - `Base#digits` integers
// - `_` digit separators
// - unquoted atoms with non-ASCII letters
// - improper lists
// - binary segment sizes
// - segment types other than `/utf8`
// - `fun` references (like `fun lists:map/2`)
// - parentheses, except between a sign and its digits (`-(1)`)
// - sigils
// - triple-quoted strings
//
// Quoted text is read as UTF-8 when the whole input is valid UTF-8. Otherwise
// each non-ASCII byte is treated as a Latin-1 character. This is the fallback
// that hex does when reading metadata.config.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use num_bigint::BigInt;

const MAX_NESTING: usize = 64;

/// An Erlang term.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Term {
    Atom(String),
    Tuple(Vec<Term>),
    List(Vec<Term>),
    /// Keys are sorted by Rust's derived `Ord` for `Term`, which is not
    /// Erlang's term order. If a key appears twice, the last value wins. This
    /// is a BTreeMap instead of a HashMap since Erlang maps can have keys that
    /// are themselves maps, which isn't possible for HashMap.
    Map(BTreeMap<Term, Term>),
    Binary(Vec<u8>),
    Integer(BigInt),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{kind} at byte {offset}")]
pub struct Error {
    pub kind: ErrorKind,
    pub offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ErrorKind {
    #[error("unexpected end of input")]
    UnexpectedEnd,
    #[error("unexpected syntax")]
    UnexpectedSyntax,
    #[error("byte value out of range")]
    ByteOutOfRange,
    #[error("nesting limit exceeded")]
    NestingLimit,
}

/// Read every term from `input`, each one ending with a `.`.
pub fn read_all(input: &[u8]) -> Result<Vec<Term>, Error> {
    let utf8 = std::str::from_utf8(input).ok();
    Parser {
        input,
        utf8,
        cursor: 0,
    }
    .parse()
}

struct Parser<'a> {
    input: &'a [u8],
    utf8: Option<&'a str>,
    cursor: usize,
}

impl Parser<'_> {
    /// Read a sequence of terms, each ending with a `.`
    fn parse(&mut self) -> Result<Vec<Term>, Error> {
        let mut terms = Vec::new();

        loop {
            self.skip_whitespace_and_comments();
            if self.cursor == self.input.len() {
                return Ok(terms);
            }

            let term = self.term(0)?;
            self.skip_whitespace_and_comments();

            if self.input.get(self.cursor) != Some(&b'.') {
                return Err(self.unexpected());
            }

            self.cursor += 1;
            terms.push(term);
        }
    }

    /// Read one term, skipping leading whitespace and comments
    fn term(&mut self, depth: usize) -> Result<Term, Error> {
        self.skip_whitespace_and_comments();
        if self.open(b"{", depth)? {
            self.separated(b"}", |parser| parser.term(depth + 1))
                .map(Term::Tuple)
        } else if self.input[self.cursor..].starts_with(b"<<") {
            self.cursor += 2;
            self.binary()
        } else if self.open(b"[", depth)? {
            self.separated(b"]", |parser| parser.term(depth + 1))
                .map(Term::List)
        } else if self.open(b"#", depth)? {
            self.skip_whitespace_and_comments();
            if self.input.get(self.cursor) != Some(&b'{') {
                return Err(self.unexpected());
            }

            self.cursor += 1;
            self.map(depth + 1)
        } else if self.input.get(self.cursor) == Some(&b'"') {
            let text = self.adjacent_strings()?;
            let charlist = Term::List(
                text.chars()
                    .map(|character| Term::Integer(BigInt::from(u32::from(character))))
                    .collect(),
            );
            Ok(charlist)
        } else if self.input.get(self.cursor) == Some(&b'\'') {
            self.quoted_text().map(Term::Atom)
        } else if self
            .input
            .get(self.cursor)
            .is_some_and(|byte| matches!(byte, b'-' | b'+') || byte.is_ascii_digit())
        {
            self.integer()
        } else {
            self.atom_name().map(Term::Atom)
        }
    }

    /// Consume an opener if present, enforcing the nesting limit
    fn open(&mut self, opener: &[u8], depth: usize) -> Result<bool, Error> {
        if !self.input[self.cursor..].starts_with(opener) {
            return Ok(false);
        }

        if depth >= MAX_NESTING {
            return Err(self.error(ErrorKind::NestingLimit));
        }

        self.cursor += opener.len();
        Ok(true)
    }

    /// Read comma-separated elements through `close`, allowing an empty
    /// sequence
    fn separated<T>(
        &mut self,
        close: &[u8],
        mut element: impl FnMut(&mut Self) -> Result<T, Error>,
    ) -> Result<Vec<T>, Error> {
        let mut items = Vec::new();
        self.skip_whitespace_and_comments();
        if self.input[self.cursor..].starts_with(close) {
            self.cursor += close.len();
            return Ok(items);
        }

        loop {
            items.push(element(self)?);
            self.skip_whitespace_and_comments();

            if self.input[self.cursor..].starts_with(close) {
                self.cursor += close.len();
                return Ok(items);
            }

            if self.input.get(self.cursor) != Some(&b',') {
                return Err(self.unexpected());
            }

            self.cursor += 1;
        }
    }

    /// Read `Key => Value` pairs through `}`, after the opening `#{`
    fn map(&mut self, depth: usize) -> Result<Term, Error> {
        let pairs = self.separated(b"}", |parser| {
            let key = parser.term(depth)?;
            parser.skip_whitespace_and_comments();
            if !parser.input[parser.cursor..].starts_with(b"=>") {
                return Err(parser.unexpected());
            }

            parser.cursor += 2;
            Ok((key, parser.term(depth)?))
        })?;

        let mut entries = BTreeMap::new();
        for (key, value) in pairs {
            let _ = entries.insert(key, value);
        }

        Ok(Term::Map(entries))
    }

    /// Read byte values and quoted segments through `>>`, after the opening `<<`
    fn binary(&mut self) -> Result<Term, Error> {
        let mut bytes = Vec::new();
        self.skip_whitespace_and_comments();
        if self.input[self.cursor..].starts_with(b">>") {
            self.cursor += 2;
            return Ok(Term::Binary(bytes));
        }

        loop {
            if self.input.get(self.cursor) == Some(&b'"') {
                self.quoted_segment(&mut bytes)?;
            } else {
                bytes.push(self.byte_value()?);
            }
            self.skip_whitespace_and_comments();

            if self.input[self.cursor..].starts_with(b">>") {
                self.cursor += 2;
                return Ok(Term::Binary(bytes));
            }

            if self.input.get(self.cursor) != Some(&b',') {
                return Err(self.unexpected());
            }

            self.cursor += 1;
            self.skip_whitespace_and_comments();
        }
    }

    /// Read an unsigned decimal integer in the range 0-255
    fn byte_value(&mut self) -> Result<u8, Error> {
        self.skip_whitespace_and_comments();
        let start = self.cursor;
        if !self.input.get(self.cursor).is_some_and(u8::is_ascii_digit) {
            return Err(self.unexpected());
        }

        let mut value = 0_u16;
        while let Some(byte @ b'0'..=b'9') = self.input.get(self.cursor) {
            value = value
                .saturating_mul(10)
                .saturating_add(u16::from(*byte - b'0'));
            self.cursor += 1;
        }

        u8::try_from(value).map_err(|_| Error {
            kind: ErrorKind::ByteOutOfRange,
            offset: start,
        })
    }

    /// Append adjacent strings as bytes, with an optional `/utf8` modifier
    fn quoted_segment(&mut self, bytes: &mut Vec<u8>) -> Result<(), Error> {
        self.skip_whitespace_and_comments();
        if self.input.get(self.cursor) != Some(&b'"') {
            return Err(self.unexpected());
        }

        // `"a" "b"/utf8` is one segment, so the modifier covers both strings
        let text = self.adjacent_strings()?;
        if self.input.get(self.cursor) != Some(&b'/') {
            // Without `/utf8`, Erlang keeps only the low byte of each character
            // instead of encoding it as UTF-8
            bytes.extend(text.chars().map(|character| character as u8));
            return Ok(());
        }

        self.cursor += 1;
        self.skip_whitespace_and_comments();
        let modifier_start = self.cursor;
        if self.atom_name()? == "utf8" {
            bytes.extend_from_slice(text.as_bytes());
            Ok(())
        } else {
            Err(Error {
                kind: ErrorKind::UnexpectedSyntax,
                offset: modifier_start,
            })
        }
    }

    /// Read decimal digits with an optional sign
    fn integer(&mut self) -> Result<Term, Error> {
        let sign = self.input.get(self.cursor).copied();
        let negative = sign == Some(b'-');
        let mut parens = 0;
        if matches!(sign, Some(b'-' | b'+')) {
            self.cursor += 1;
            self.skip_whitespace_and_comments();

            while self.input.get(self.cursor) == Some(&b'(') {
                parens += 1;
                self.cursor += 1;
                self.skip_whitespace_and_comments();
            }
        }

        let start = self.cursor;
        while self.input.get(self.cursor).is_some_and(u8::is_ascii_digit) {
            self.cursor += 1;
        }

        if self.cursor == start {
            return Err(self.unexpected());
        }

        // 1.0 is a float, not the integer 1` followed by a form terminator
        if self.input.get(self.cursor) == Some(&b'.')
            && self
                .input
                .get(self.cursor + 1)
                .is_some_and(u8::is_ascii_digit)
        {
            return Err(self.unexpected());
        }

        let value = BigInt::parse_bytes(&self.input[start..self.cursor], 10)
            .expect("decimal digits parse as an integer");

        for _ in 0..parens {
            self.skip_whitespace_and_comments();
            if self.input.get(self.cursor) != Some(&b')') {
                return Err(self.unexpected());
            }

            self.cursor += 1;
        }

        Ok(Term::Integer(if negative { -value } else { value }))
    }

    /// Join quoted strings separated by whitespace or comments, such as
    /// `"a" "b"`
    fn adjacent_strings(&mut self) -> Result<String, Error> {
        let mut text = self.quoted_text()?;
        self.skip_whitespace_and_comments();
        while self.input.get(self.cursor) == Some(&b'"') {
            text.push_str(&self.quoted_text()?);
            self.skip_whitespace_and_comments();
        }

        Ok(text)
    }

    /// Read quoted text and decode its escapes, starting at the opening quote
    fn quoted_text(&mut self) -> Result<String, Error> {
        let quote = self.input[self.cursor];
        self.cursor += 1;
        let mut text = String::new();

        loop {
            match self.input.get(self.cursor) {
                Some(byte) if *byte == quote => {
                    self.cursor += 1;
                    return Ok(text);
                }
                Some(b'\\') => {
                    self.cursor += 1;
                    text.push(self.escaped_character()?);
                }
                Some(_) => {
                    let start = self.cursor;
                    while self
                        .input
                        .get(self.cursor)
                        .is_some_and(|byte| byte.is_ascii() && *byte != quote && *byte != b'\\')
                    {
                        self.cursor += 1;
                    }

                    if self.cursor > start {
                        text.push_str(
                            std::str::from_utf8(&self.input[start..self.cursor])
                                .expect("ASCII text run"),
                        );
                    } else {
                        text.push(self.next_character().expect("input byte is present"));
                    }
                }
                None => return Err(self.error(ErrorKind::UnexpectedEnd)),
            }
        }
    }

    /// Read an octal, hex, control, or named escape after the backslash
    fn escaped_character(&mut self) -> Result<char, Error> {
        let digits = self.input[self.cursor..]
            .iter()
            .take(3)
            .take_while(|byte| (b'0'..=b'7').contains(*byte))
            .count();
        if digits > 0 {
            let value = self.input[self.cursor..self.cursor + digits]
                .iter()
                .fold(0, |value, digit| value * 8 + u32::from(digit - b'0'));
            self.cursor += digits;
            return Ok(char::from_u32(value).expect("octal escape is a valid character"));
        }

        let escaped = match self.next_character() {
            Some(character) => character,
            None => return Err(self.error(ErrorKind::UnexpectedEnd)),
        };
        let character = match escaped {
            // Hex reads every `\x` as a literal `x`, even a well-formed one.
            // We deliberately decode well-formed escapes the way Erlang does
            // instead, and only fall back to Hex's literal `x` for malformed
            // ones so we don't reject metadata that hex.pm accepted.
            'x' => self.hex_escape().unwrap_or('x'),
            '^' => match self.next_character() {
                Some('?') => '\u{007f}',
                Some(control) => {
                    char::from_u32(u32::from(control) & 31).expect("control escape is a character")
                }
                None => return Err(self.error(ErrorKind::UnexpectedEnd)),
            },
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\u{000b}',
            'b' => '\u{0008}',
            'f' => '\u{000c}',
            'e' => '\u{001b}',
            's' => ' ',
            'd' => '\u{007f}',
            character => character,
        };
        Ok(character)
    }

    /// `\xHH` takes exactly two hex digits and `\x{H...}` any number
    fn hex_escape(&mut self) -> Option<char> {
        let rest = &self.input[self.cursor..];
        let (digits, consumed) = if rest.first() == Some(&b'{') {
            let length = rest[1..]
                .iter()
                .take_while(|byte| byte.is_ascii_hexdigit())
                .count();
            if length == 0 || rest.get(length + 1) != Some(&b'}') {
                return None;
            }
            (&rest[1..=length], length + 2)
        } else if rest.len() >= 2 && rest[..2].iter().all(u8::is_ascii_hexdigit) {
            (&rest[..2], 2)
        } else {
            return None;
        };

        let value = std::str::from_utf8(digits)
            .ok()
            .and_then(|digits| u32::from_str_radix(digits, 16).ok())
            .and_then(char::from_u32)?;
        self.cursor += consumed;
        Some(value)
    }

    /// Read an ASCII atom
    fn atom_name(&mut self) -> Result<String, Error> {
        let start = self.cursor;
        if !self
            .input
            .get(self.cursor)
            .is_some_and(u8::is_ascii_lowercase)
        {
            return Err(self.unexpected());
        }
        self.cursor += 1;

        while self
            .input
            .get(self.cursor)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'@')
        {
            self.cursor += 1;
        }

        Ok(std::str::from_utf8(&self.input[start..self.cursor])
            .expect("atom syntax is ASCII")
            .to_owned())
    }

    /// Read one character using UTF-8, or Latin-1 when the input is invalid
    /// UTF-8
    fn next_character(&mut self) -> Option<char> {
        let byte = *self.input.get(self.cursor)?;
        if byte.is_ascii() {
            self.cursor += 1;
            Some(char::from(byte))
        } else if let Some(utf8) = self.utf8 {
            let character = utf8[self.cursor..]
                .chars()
                .next()
                .expect("UTF-8 cursor points to a character");
            self.cursor += character.len_utf8();
            Some(character)
        } else {
            self.cursor += 1;
            Some(char::from(byte))
        }
    }

    /// Skip bytes through ASCII space and `%` comments through the end of the
    /// line
    fn skip_whitespace_and_comments(&mut self) {
        loop {
            if self
                .input
                .get(self.cursor)
                .is_some_and(|byte| *byte <= b' ')
            {
                self.cursor += 1;
            } else if self.input.get(self.cursor) == Some(&b'%') {
                self.cursor += 1;
                while self
                    .input
                    .get(self.cursor)
                    .is_some_and(|byte| *byte != b'\n')
                {
                    self.cursor += 1;
                }
            } else {
                return;
            }
        }
    }

    fn unexpected(&self) -> Error {
        if self.cursor == self.input.len() {
            self.error(ErrorKind::UnexpectedEnd)
        } else {
            self.error(ErrorKind::UnexpectedSyntax)
        }
    }

    fn error(&self, kind: ErrorKind) -> Error {
        Error {
            kind,
            offset: self.cursor,
        }
    }
}
