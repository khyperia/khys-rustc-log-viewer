use bumpalo::Bump;
use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    str::Chars,
};

use crate::HopMessageKind;

#[derive(Debug)]
pub struct ParseError {
    line: String,
    message: String,
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        write!(f, "parse error: {} -- on line {}", self.message, self.line)
    }
}

impl Error for ParseError {}

pub fn custom_parse<'b>(bump: &'b Bump, s: &str) -> Result<ParsedMessage<'b>, ParseError> {
    custom_parse2(bump, &mut s.chars()).map_err(|inner| ParseError {
        line: s.to_string(),
        message: inner.to_string(),
    })
}

enum InnerParseError<'b> {
    UnknownField(&'b str),
    UnexpectedEof(&'static str),
    UnexpectedChar(&'static [char], char),
    UnexpectedCharSingle(char, char),
}

impl Display for InnerParseError<'_> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Self::UnknownField(field) => write!(f, "unknown field {field}"),
            Self::UnexpectedEof(kind) => write!(f, "unexpected eof parsing {kind}"),
            Self::UnexpectedChar(expected, got) => write!(
                f,
                "unexpected character '{got}' (expected one of {expected:?})"
            ),
            Self::UnexpectedCharSingle(expected, got) => {
                write!(f, "unexpected character '{got}' (expected '{expected}')")
            }
        }
    }
}

fn custom_parse2<'b>(
    bump: &'b Bump,
    iter: &mut Chars,
) -> Result<ParsedMessage<'b>, InnerParseError<'b>> {
    let mut result = ParsedMessage::default();

    expect(iter, '{')?;

    loop {
        let key = parse_string(bump, iter)?;
        expect(iter, ':')?;
        match key {
            "timestamp" => result.timestamp = parse_string(bump, iter)?,
            "level" => result.level = parse_string(bump, iter)?,
            "fields" => parse_dict(bump, iter, &mut result.fields, |_, _| false)?,
            "target" => result.target = parse_string(bump, iter)?,
            "filename" => result.filename = parse_string(bump, iter)?,
            "line_number" => result.line_number = parse_number(bump, iter)?,
            "span" => parse_span(bump, iter, &mut result.span)?,
            "spans" => parse_spans(bump, iter, &mut result.spans)?,
            _ => break Err(InnerParseError::UnknownField(key)),
        }
        match iter
            .next()
            .ok_or(InnerParseError::UnexpectedEof("message"))?
        {
            '}' => break Ok(result),
            ',' => (),
            ch => break Err(InnerParseError::UnexpectedChar(&['}', ','], ch)),
        }
    }
}

fn parse_spans<'b>(
    bump: &'b Bump,
    iter: &mut Chars,
    result: &mut &'b LinkedList<'b, Span<'b>>,
) -> Result<(), InnerParseError<'b>> {
    expect(iter, '[')?;
    let mut iter_clone = iter.clone();
    if iter_clone.next() == Some(']') {
        *iter = iter_clone;
        return Ok(());
    }
    loop {
        let mut single = Span::default();
        parse_span(bump, iter, &mut single)?;
        let cons = LinkedList::Cons {
            next: result,
            item: single,
        };
        *result = bump.alloc(cons);
        match iter
            .next()
            .ok_or(InnerParseError::UnexpectedEof("dict array"))?
        {
            ']' => break Ok(()),
            ',' => (),
            ch => break Err(InnerParseError::UnexpectedChar(&[']', ','], ch)),
        }
    }
}

fn parse_span<'b>(
    bump: &'b Bump,
    iter: &mut Chars,
    result: &mut Span<'b>,
) -> Result<(), InnerParseError<'b>> {
    parse_dict(bump, iter, &mut result.map, |key, value| {
        if key == "name" {
            result.name = Some(value);
            true
        } else {
            false
        }
    })
}

fn parse_dict<'b>(
    bump: &'b Bump,
    iter: &mut Chars,
    result: &mut &'b JsonMap<'b>,
    mut on_key_value: impl FnMut(&'b str, &'b str) -> bool,
) -> Result<(), InnerParseError<'b>> {
    expect(iter, '{')?;
    loop {
        let key = parse_string(bump, iter)?;
        expect(iter, ':')?;
        let start = iter.as_str();
        let value = match iter
            .next()
            .ok_or(InnerParseError::UnexpectedEof("dict key"))?
        {
            '"' => parse_string_after_quote(bump, iter)?,
            't' => {
                expect(iter, 'r')?;
                expect(iter, 'u')?;
                expect(iter, 'e')?;
                "true"
            }
            'f' => {
                expect(iter, 'a')?;
                expect(iter, 'l')?;
                expect(iter, 's')?;
                expect(iter, 'e')?;
                "false"
            }
            ch if ch.is_ascii_digit() => parse_number_after_digit(bump, start, iter),
            ch => break Err(InnerParseError::UnexpectedChar(&['"', 't', 'f', '0'], ch)),
        };
        if !on_key_value(key, value) {
            let cons = JsonMap::Cons {
                next: result,
                item: (key, value),
            };
            *result = bump.alloc(cons);
        }
        match iter.next().ok_or(InnerParseError::UnexpectedEof("dict"))? {
            '}' => break Ok(()),
            ',' => (),
            ch => break Err(InnerParseError::UnexpectedChar(&['}', ','], ch)),
        }
    }
}

fn parse_string<'b>(bump: &'b Bump, iter: &mut Chars) -> Result<&'b str, InnerParseError<'b>> {
    expect(iter, '"')?;
    parse_string_after_quote(bump, iter)
}

fn parse_string_after_quote<'b>(
    bump: &'b Bump,
    iter: &mut Chars,
) -> Result<&'b str, InnerParseError<'b>> {
    let str_begin = iter.as_str();
    let mut backslash_count = 0;
    let mut index = 0;
    let end_index = loop {
        if let Some(tmp) = str_begin[index..].find(['"', '\\']) {
            index += tmp;
        } else {
            return Err(InnerParseError::UnexpectedEof("string"));
        }
        if str_begin.as_bytes()[index] == b'\\' {
            backslash_count += 1;
            let mut char_skipper = str_begin[(index + 1)..].chars();
            // skip the escaped char
            if char_skipper.next().is_none() {
                return Err(InnerParseError::UnexpectedEof("string backslash"));
            }
            index = str_begin.len() - char_skipper.as_str().len();
        } else {
            *iter = str_begin[(index + 1)..].chars();
            if backslash_count == 0 {
                // There are no backslashes. Use a direct bumpalo alloc_str to copy the data into
                // bumpalo.
                return Ok(bump.alloc_str(&str_begin[..index]));
            }
            // There are backslashes. Do a custom alloc_slice_fill_with to skip over the
            // backslashes while copying.
            break index;
        }
    };

    let mut i = 0;
    let fullslice = &str_begin.as_bytes()[..end_index];
    let result = bump.alloc_slice_fill_with(end_index - backslash_count, |_| {
        let mut result = fullslice[i];
        // backslash is a single ascii character, so it can be detected as a u8
        if result == b'\\' {
            i += 1;
            result = match fullslice[i] {
                b'n' => b'\n',
                b'r' => b'\r',
                ch => ch,
            };
        }
        i += 1;
        result
    });
    debug_assert!(i == fullslice.len());
    // from_utf8_unchecked does not seem to improve perf much, even though it would be
    // theoretically safe to do so
    Ok(str::from_utf8(result).unwrap())
}

fn parse_number<'b>(bump: &'b Bump, iter: &mut Chars) -> Result<&'b str, InnerParseError<'static>> {
    let start = iter.as_str();
    let ch = iter
        .next()
        .ok_or(InnerParseError::UnexpectedEof("number"))?;
    if ch.is_ascii_digit() {
        Ok(parse_number_after_digit(bump, start, iter))
    } else {
        Err(InnerParseError::UnexpectedChar(&['0'], ch))
    }
}

fn parse_number_after_digit<'b>(bump: &'b Bump, start: &str, iter: &mut Chars) -> &'b str {
    let slice = loop {
        let mut cloned = iter.clone();
        let Some(ch) = cloned.next() else {
            break start;
        };
        if !ch.is_ascii_digit() {
            let len = start.len() - iter.as_str().len();
            break &start[..len];
        }
        *iter = cloned;
    };
    bump.alloc_str(slice)
}

fn expect(iter: &mut Chars, ch: char) -> Result<(), InnerParseError<'static>> {
    if let Some(got) = iter.next() {
        if got == ch {
            Ok(())
        } else {
            Err(InnerParseError::UnexpectedCharSingle(ch, got))
        }
    } else {
        Err(InnerParseError::UnexpectedCharSingle(ch, '\0'))
    }
}

#[derive(Default)]
pub struct ParsedMessage<'b> {
    pub timestamp: &'b str,
    pub level: &'b str,
    pub fields: &'b JsonMap<'b>,
    pub target: &'b str,
    pub filename: &'b str,
    pub line_number: &'b str,
    pub span: Span<'b>,
    pub spans: &'b LinkedList<'b, Span<'b>>,
}

impl ParsedMessage<'_> {
    pub fn hop_message(&self) -> Option<HopMessageKind> {
        if let JsonMap::Cons {
            next: JsonMap::Empty,
            item: ("message", value),
        } = *self.fields
        {
            match value {
                "enter" => Some(HopMessageKind::Enter),
                "exit" => Some(HopMessageKind::Exit),
                _ => None,
            }
        } else {
            None
        }
    }
}

#[derive(Default)]
pub struct Span<'b> {
    pub name: Option<&'b str>,
    pub map: &'b JsonMap<'b>,
}

pub enum LinkedList<'b, T> {
    Empty,
    Cons {
        next: &'b LinkedList<'b, T>,
        item: T,
    },
}

pub type JsonMap<'b> = LinkedList<'b, (&'b str, &'b str)>;

impl<'b, T> LinkedList<'b, T> {
    pub fn iter(&'b self) -> LinkedListIter<'b, T> {
        LinkedListIter { entry: self }
    }
}

impl<'b, T> Default for &'b LinkedList<'b, T> {
    fn default() -> Self {
        &LinkedList::Empty
    }
}

pub struct LinkedListIter<'b, T> {
    entry: &'b LinkedList<'b, T>,
}

impl<'b, T> Iterator for LinkedListIter<'b, T> {
    type Item = &'b T;
    fn next(&mut self) -> Option<Self::Item> {
        match self.entry {
            LinkedList::Empty => None,
            LinkedList::Cons { next, item } => {
                self.entry = *next;
                Some(item)
            }
        }
    }
}
