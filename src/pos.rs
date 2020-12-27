//! Source code locations (some parts borrowed from [qluon])
//!
//! [qluon]: https://github.com/gluon-lang/gluon/blob/master/base/src/pos.rs

use std::fmt;
use std::borrow::Cow;
use std::num::NonZeroU32;

pub type FileId = Option<NonZeroU32>;
use codespan_reporting::{files as cs_files};
pub use codespan::{ByteIndex as BytePos, ByteOffset, RawIndex, RawOffset};

pub type Files = NonUtf8Files;

/// An implementation of [`codespan_reporting::files::Files`] adapted to non-UTF8 files.
#[derive(Debug, Clone)]
pub struct NonUtf8Files {
    inner: cs_files::SimpleFiles<String, String>,
}

impl NonUtf8Files {
    pub fn new() -> Self { NonUtf8Files { inner: cs_files::SimpleFiles::new() } }

    pub fn add(&mut self, name: &str, source: &[u8]) -> FileId {
        Self::shift_file_id(self.inner.add(
            name.to_owned(),
            prepare_diagnostic_text_source(source).into(),
        ))
    }

    /// Convenience method to parse a piece of code in a way that ensures that the `Span`s will
    /// be available for diagnostic rendering.
    pub fn parse<'input, T>(&mut self, filename: &str, source: &'input [u8])
        -> crate::parse::Result<'input, Spanned<T>>
    where
        T: crate::Parse<'input>,
    {
        let file_id = self.add(filename, source.as_ref());
        let mut state = crate::parse::State::new(file_id);
        T::parse_stream(&mut state, crate::parse::lexer::Lexer::new(source.as_ref()))
    }

    fn unshift_file_id(file_id: FileId) -> Result<usize, cs_files::Error> {
        // produce Error on file_id = None; such spans aren't fit for diagnostics
        let file_id: u32 = file_id.ok_or(cs_files::Error::FileMissing)?.into();
        Ok(file_id as usize - 1)
    }

    fn shift_file_id(file_id: usize) -> FileId {
        NonZeroU32::new(file_id as u32 + 1)
    }
}

/// This implementation provides source text that has been lossily modified to be valid UTF-8,
/// and which should only be used for diagnostic purposes.
impl<'a> cs_files::Files<'a> for NonUtf8Files {
    type FileId = FileId;
    type Name = String;
    type Source = &'a str;

    // Just delegate everything
    fn name(&self, file_id: FileId) -> Result<String, cs_files::Error> {
        self.inner.name(Self::unshift_file_id(file_id)?)
    }

    fn source(&self, file_id: FileId) -> Result<&str, cs_files::Error> {
        self.inner.source(Self::unshift_file_id(file_id)?)
    }

    fn line_index(&self, file_id: FileId, byte_index: usize) -> Result<usize, cs_files::Error> {
        self.inner.line_index(Self::unshift_file_id(file_id)?, byte_index)
    }
    fn line_range(&self, file_id: FileId, line_index: usize) -> Result<std::ops::Range<usize>, cs_files::Error> {
        self.inner.line_range(Self::unshift_file_id(file_id)?, line_index)
    }
}

/// A version of `from_utf8_lossy` that preserves byte positions.
///
/// The output of this is suitable for rendering spans in error messages.
///
/// It accomplishes this by using `?` as the replacement character, which only takes a single byte
/// and can thus easily fill arbitrarily-sized spaces, unlike `U+FFFD REPLACEMENT CHARACTER`
/// which takes three bytes.
fn prepare_diagnostic_text_source(s: &[u8]) -> Cow<str> {
    match std::str::from_utf8(s) {
        Ok(valid) => Cow::Borrowed(valid),
        Err(error) => {
            let mut remaining = s;
            let mut out = String::new();
            let mut res = Err(error);
            while let Err(error) = res {
                let (valid, after_valid) = remaining.split_at(error.valid_up_to());
                out.push_str(std::str::from_utf8(valid).expect("already validated"));

                let num_bad = error.error_len().unwrap_or(after_valid.len());
                for _ in 0..num_bad {
                    out.push('?');
                }
                remaining = &after_valid[num_bad..];
                res = std::str::from_utf8(remaining);
            }
            match res {
                Err(_) => unreachable!(),
                Ok(remaining_str) => out.push_str(remaining_str),
            }
            assert_eq!(s.len(), out.len());
            Cow::Owned(out)
        },
    }
}

#[test]
fn test_lossy_utf8() {
    let func = prepare_diagnostic_text_source;

    // valid UTF-8
    assert_eq!(func(b"ab\xF0\x9F\x92\x96cd"), "ab💖cd");

    // invalid byte sequence...
    assert_eq!(func(b"\x80\xFFcd"), "??cd"); // ...at beginning
    assert_eq!(func(b"ab\x80\xFFcd"), "ab??cd"); // ...in middle
    assert_eq!(func(b"ab\x80\xFF"), "ab??"); // ...at end

    // incomplete character; byte 0b11110000 expects 3 more bytes after it.
    // (this is the case where Utf8Error::error_len() returns None)
    assert_eq!(func(b"ab\xF0\x80\x80"), "ab???");

    // unpaired surrogate
    // http://simonsapin.github.io/wtf-8/#surrogates-byte-sequences
    assert_eq!(func(b"ab\xED\xA3\xA4cd"), "ab???cd");

    // ambiguous case.  This begins with a 4-byte character starter byte, but returns to ascii after
    // 2 bytes. I'm not sure whether the documentation of `Utf8Error::error_len` is specified
    // well-enough to determine whether this would replace the two 'w' characters.
    let input = b"ab\xF0\x80wwcd";
    let output = func(input);
    assert_eq!(output.len(), input.len());
    assert_eq!(&output.as_bytes()[..2], &input[..2]);
    assert_eq!(&output.as_bytes()[2..2+2], b"??");
    assert_eq!(&output.as_bytes()[2+4..], &input[2+4..]);
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub start: BytePos,
    pub end: BytePos,
    // FIXME: This is somewhat undesirable as it gets repeated all over the place.
    //        Gluon seems to have some way of making byte indices work as FileIds,
    //        but something seemed off about their Files impl when I tried it...
    pub file_id: FileId,
}

impl Span {
    /// Create a new span from a starting and ending span.
    pub fn new(file_id: FileId, start: impl Into<BytePos>, end: impl Into<BytePos>) -> Span {
        let start = start.into();
        let end = end.into();
        assert!(end >= start);

        Span { file_id, start, end }
    }

    /// Gives an empty span at the start of a source.
    pub const fn initial(file_id: FileId) -> Span {
        Span {
            file_id,
            start: BytePos(0),
            end: BytePos(0),
        }
    }

    /// Measure the span of a string.
    ///
    /// ```rust
    /// use codespan::{ByteIndex, Span};
    ///
    /// let span = Span::from_str("hello");
    ///
    /// assert_eq!(span, Span::new(0, 5));
    /// ```
    pub fn from_str(s: &str) -> Span {
        Span::new(None, 0, s.len() as RawIndex)
    }

    /// Combine two spans by taking the start of the earlier span
    /// and the end of the later span.
    ///
    /// Note: this will work even if the two spans are disjoint.
    /// If this doesn't make sense in your application, you should handle it yourself.
    /// In that case, you can use `Span::disjoint` as a convenience function.
    ///
    /// ```rust
    /// use codespan::Span;
    ///
    /// let span1 = Span::from(0..4);
    /// let span2 = Span::from(10..16);
    ///
    /// assert_eq!(Span::merge(span1, span2), Span::from(0..16));
    /// ```
    pub fn merge(self, other: Span) -> Span {
        use std::cmp::{max, min};

        assert_eq!(self.file_id, other.file_id);
        let start = min(self.start, other.start);
        let end = max(self.end, other.end);
        Span::new(self.file_id, start, end)
    }

    /// A helper function to tell whether two spans do not overlap.
    ///
    /// ```
    /// use ecl_parser::pos::{Span};
    /// let span1 = Span::from(0..4);
    /// let span2 = Span::from(10..16);
    /// assert!(span1.disjoint(span2));
    /// ```
    pub fn disjoint(self, other: Span) -> bool {
        assert_eq!(self.file_id.is_some(), other.file_id.is_some(), "can't compare dummy file span to non-dummy");
        if self.file_id != other.file_id {
            return true;
        }
        let (first, last) = if self.end < other.end {
            (self, other)
        } else {
            (other, self)
        };
        first.end <= last.start
    }

    /// Get the starting byte index.
    ///
    /// ```rust
    /// use ecl_parser::pos::{BytePos, Span};
    ///
    /// let span = Span::new(None, 0, 4);
    ///
    /// assert_eq!(span.start(), BytePos::from(0));
    /// ```
    pub fn start(self) -> BytePos {
        self.start
    }

    /// Get the ending byte index.
    ///
    /// ```rust
    /// use ecl_parser::pos::{BytePos, Span};
    ///
    /// let span = Span::new(None, 0, 4);
    ///
    /// assert_eq!(span.end(), BytePos::from(4));
    /// ```
    pub fn end(self) -> BytePos {
        self.end
    }
}

impl Default for Span {
    fn default() -> Span {
        Span::initial(None)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{start}, {end})",
            start = self.start(),
            end = self.end(),
        )
    }
}

impl<I> From<std::ops::Range<I>> for Span
where
    I: Into<BytePos>,
{
    fn from(range: std::ops::Range<I>) -> Span {
        Span::new(None, range.start, range.end)
    }
}

impl From<Span> for std::ops::Range<usize> {
    fn from(span: Span) -> std::ops::Range<usize> {
        span.start.into()..span.end.into()
    }
}

impl From<Span> for std::ops::Range<RawIndex> {
    fn from(span: Span) -> std::ops::Range<RawIndex> {
        span.start.0..span.end.0
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn test_merge() {
        use super::Span;

        // overlap
        let a = Span::from(1..5);
        let b = Span::from(3..10);
        assert_eq!(a.merge(b), Span::from(1..10));
        assert_eq!(b.merge(a), Span::from(1..10));

        // subset
        let two_four = (2..4).into();
        assert_eq!(a.merge(two_four), (1..5).into());
        assert_eq!(two_four.merge(a), (1..5).into());

        // disjoint
        let ten_twenty = (10..20).into();
        assert_eq!(a.merge(ten_twenty), (1..20).into());
        assert_eq!(ten_twenty.merge(a), (1..20).into());

        // identity
        assert_eq!(a.merge(a), a);
    }

    #[test]
    fn test_disjoint() {
        use super::Span;

        // overlap
        let a = Span::from(1..5);
        let b = Span::from(3..10);
        assert!(!a.disjoint(b));
        assert!(!b.disjoint(a));

        // subset
        let two_four = (2..4).into();
        assert!(!a.disjoint(two_four));
        assert!(!two_four.disjoint(a));

        // disjoint
        let ten_twenty = (10..20).into();
        assert!(a.disjoint(ten_twenty));
        assert!(ten_twenty.disjoint(a));

        // identity
        assert!(!a.disjoint(a));

        // off by one (upper bound)
        let c = Span::from(5..10);
        assert!(a.disjoint(c));
        assert!(c.disjoint(a));
        // off by one (lower bound)
        let d = Span::from(0..1);
        assert!(a.disjoint(d));
        assert!(d.disjoint(a));
    }
}


/// An AST node with a span.  The span is not included in comparisons or hashes.
#[derive(Copy, Clone, Default)]
pub struct Spanned<T: ?Sized> {
    pub span: Span,
    pub value: T,
}

impl<T: fmt::Debug> fmt::Debug for Spanned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Spanned")
            // format as a range instead of Span's derived Debug
            .field("span", &(self.span.start().0..self.span.end().0))
            .field("value", &self.value)
            .finish()
    }
}

impl<T> Spanned<T> {
    pub fn null_from<U: Into<T>>(value: U) -> Self {
        Spanned {
            span: Span::default(),
            value: value.into(),
        }
    }

    pub fn new_from<U: Into<T>>(span: Span, value: U) -> Self {
        Spanned { span, value: value.into() }
    }
}

impl<T> From<T> for Spanned<T> {
    fn from(value: T) -> Self {
        Spanned {
            span: Span::default(),
            value,
        }
    }
}

impl<T: ?Sized + Eq> Eq for Spanned<T> {}

impl<T: ?Sized + PartialEq> PartialEq for Spanned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T: ?Sized + PartialEq> PartialEq<T> for Spanned<T> {
    fn eq(&self, other: &T) -> bool {
        self.value == *other
    }
}

impl<T: ?Sized + std::hash::Hash> std::hash::Hash for Spanned<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<T: ?Sized> std::ops::Deref for Spanned<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T: ?Sized> std::ops::DerefMut for Spanned<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.value
    }
}

impl<T: ?Sized, U: ?Sized> AsRef<U> for Spanned<T>
where
    T: AsRef<U>,
{
    fn as_ref(&self) -> &U {
        self.value.as_ref()
    }
}

impl<T> Spanned<T> {
    pub fn map<U>(self, mut f: impl FnMut(T) -> U) -> Spanned<U> {
        Spanned {
            span: self.span,
            value: f(self.value),
        }
    }
}

impl<T: ?Sized + fmt::Display> fmt::Display for Spanned<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", &self.value)
    }
}
