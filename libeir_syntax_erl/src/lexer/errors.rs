use std::hash::{Hash, Hasher};

use snafu::Snafu;

use libeir_diagnostics::{Diagnostic, Label, SourceIndex, SourceSpan};

use super::token::{Token, TokenType};

/// An enum of possible errors that can occur during lexing.
#[derive(Clone, Debug, PartialEq, Snafu)]
pub enum LexicalError {
    #[snafu(display("{}", reason))]
    InvalidFloat { span: SourceSpan, reason: String },

    #[snafu(display("{}", reason))]
    InvalidRadix { span: SourceSpan, reason: String },

    /// Occurs when a string literal is not closed (e.g. `"this is an unclosed string`)
    /// It is also implicit that hitting this error means we've reached EOF, as we'll scan the
    /// entire input looking for the closing quote
    #[snafu(display("Unclosed string literal"))]
    UnclosedString { span: SourceSpan },

    /// Like UnclosedStringLiteral, but for quoted atoms
    #[snafu(display("Unclosed atom literal"))]
    UnclosedAtom { span: SourceSpan },

    /// Occurs when an escape sequence is encountered but the code is unsupported or unrecognized
    #[snafu(display("{}", reason))]
    InvalidEscape { span: SourceSpan, reason: String },

    /// Occurs when we encounter an unexpected character
    #[snafu(display("Encountered unexpected character '{}'", found))]
    UnexpectedCharacter { start: SourceIndex, found: char },
}
impl Hash for LexicalError {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let id = match *self {
            LexicalError::InvalidFloat { .. } => 0,
            LexicalError::InvalidRadix { .. } => 1,
            LexicalError::UnclosedString { .. } => 2,
            LexicalError::UnclosedAtom { .. } => 3,
            LexicalError::InvalidEscape { .. } => 4,
            LexicalError::UnexpectedCharacter { .. } => 5,
        };
        id.hash(state);
    }
}
impl LexicalError {
    /// Return the source span for this error
    pub fn span(&self) -> SourceSpan {
        match *self {
            LexicalError::InvalidFloat { span, .. } => span,
            LexicalError::InvalidRadix { span, .. } => span,
            LexicalError::UnclosedString { span, .. } => span,
            LexicalError::UnclosedAtom { span, .. } => span,
            LexicalError::InvalidEscape { span, .. } => span,
            LexicalError::UnexpectedCharacter { start, .. } => SourceSpan::new(start, start),
        }
    }

    /// Get diagnostic for display
    pub fn to_diagnostic(&self) -> Diagnostic {
        let span = self.span();
        let msg = self.to_string();
        match *self {
            LexicalError::InvalidFloat { .. } => Diagnostic::error()
                .with_message("invalid float literal")
                .with_labels(vec![
                    Label::primary(span.source_id(), span).with_message(msg)
                ]),
            LexicalError::InvalidRadix { .. } => Diagnostic::error()
                .with_message("invalid radix value for integer literal")
                .with_labels(vec![
                    Label::primary(span.source_id(), span).with_message(msg)
                ]),
            LexicalError::InvalidEscape { .. } => Diagnostic::error()
                .with_message("invalid escape sequence")
                .with_labels(vec![
                    Label::primary(span.source_id(), span).with_message(msg)
                ]),
            LexicalError::UnexpectedCharacter { .. } => Diagnostic::error()
                .with_message("unexpected character")
                .with_labels(vec![
                    Label::primary(span.source_id(), span).with_message(msg)
                ]),
            _ => Diagnostic::error()
                .with_message(msg)
                .with_labels(vec![Label::primary(span.source_id(), span)]),
        }
    }
}

// Produced when converting from LexicalToken to {Atom,Ident,String,Symbol}Token
#[derive(Debug, Clone)]
pub struct TokenConvertError {
    pub span: SourceSpan,
    pub token: Token,
    pub expected: TokenType,
}
