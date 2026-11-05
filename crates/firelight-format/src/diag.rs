//! Diagnostics: every problem with its file, line and column.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        }
    }
}

/// One problem. `path` is relative to the repo root, e.g.
/// `.firelight/guides/request-access.md`; `line` and `col` are 1-based, 0
/// when the problem has no position (a missing file, a folder-wide limit).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Diagnostic {
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub severity: Severity,
    /// A stable kebab-case code, e.g. `unknown-key`, `unresolved-ref`.
    pub code: &'static str,
    pub message: String,
}

impl Diagnostic {
    pub fn error(
        path: impl Into<String>,
        line: u32,
        col: u32,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            line,
            col,
            severity: Severity::Error,
            code,
            message: message.into(),
        }
    }

    pub fn warning(
        path: impl Into<String>,
        line: u32,
        col: u32,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            line,
            col,
            severity: Severity::Warning,
            code,
            message: message.into(),
        }
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// `path:line:col: error: message` — the format editors and CI annotations
/// understand.
impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(
                f,
                "{}:{}:{}: {}: {}",
                self.path,
                self.line,
                self.col.max(1),
                self.severity.as_str(),
                self.message
            )
        } else {
            write!(
                f,
                "{}: {}: {}",
                self.path,
                self.severity.as_str(),
                self.message
            )
        }
    }
}

/// Sort by file, then position, errors first at the same spot.
pub fn sort(diags: &mut [Diagnostic]) {
    diags.sort_by(|a, b| {
        (
            a.path.as_str(),
            a.line,
            a.col,
            a.severity,
            a.code,
            a.message.as_str(),
        )
            .cmp(&(
                b.path.as_str(),
                b.line,
                b.col,
                b.severity,
                b.code,
                b.message.as_str(),
            ))
    });
}
