use std::borrow::Cow;

use crate::{exception_public::MontyException, metadata::AnnotatedObject};

/// Output handler for the `print()` builtin function.
///
/// Provides common output modes as enum variants to avoid trait object overhead
/// in the typical cases (stdout, disabled, collect). For custom output handling,
/// use the `Callback` variant with a [`PrintWriterCallback`] implementation.
///
/// # Variants
/// - `Disabled` - Silently discards all output (useful for benchmarking or suppressing output)
/// - `Stdout` - Writes to standard output (the default behavior)
/// - `Collect` - Accumulates output into a target `String` for programmatic access
/// - `Callback` - Delegates to a user-provided [`PrintWriterCallback`] implementation
pub enum PrintWriter<'a> {
    /// Silently discard all output.
    Disabled,
    /// Write to standard output.
    Stdout,
    /// Collect all output into a string.
    Collect(&'a mut String),
    /// Delegate to a custom callback.
    Callback(&'a mut dyn PrintWriterCallback),
}

impl PrintWriter<'_> {
    /// Creates a new `PrintWriter` that reborrows the same underlying target.
    ///
    /// This is useful in iterative execution (`start`/`resume` loops) where each
    /// step takes `PrintWriter` by value but you want all steps to write to the
    /// same output target. The original writer remains valid after the reborrowed
    /// copy is dropped.
    pub fn reborrow(&mut self) -> PrintWriter<'_> {
        match self {
            Self::Disabled => PrintWriter::Disabled,
            Self::Stdout => PrintWriter::Stdout,
            Self::Collect(buf) => PrintWriter::Collect(buf),
            Self::Callback(cb) => PrintWriter::Callback(&mut **cb),
        }
    }

    /// Called once for each formatted argument passed to `print()`.
    ///
    /// This method writes only the given argument's text, without adding
    /// separators or a trailing newline. Separators (spaces) and the final
    /// terminator (newline) are emitted via [`stdout_push`](Self::stdout_push).
    pub fn stdout_write(&mut self, output: Cow<'_, str>) -> Result<(), MontyException> {
        match self {
            Self::Disabled => Ok(()),
            Self::Stdout => {
                print!("{output}");
                Ok(())
            }
            Self::Collect(buf) => {
                buf.push_str(&output);
                Ok(())
            }
            Self::Callback(cb) => cb.stdout_write(output),
        }
    }

    /// Appends a single character to the output.
    ///
    /// Generally called to add spaces (separators) and newlines (terminators)
    /// within print output.
    pub fn stdout_push(&mut self, end: char) -> Result<(), MontyException> {
        match self {
            Self::Disabled => Ok(()),
            Self::Stdout => {
                print!("{end}");
                Ok(())
            }
            Self::Collect(buf) => {
                buf.push(end);
                Ok(())
            }
            Self::Callback(cb) => cb.stdout_push(end),
        }
    }

    /// Returns `true` if this writer wants structured output via
    /// [`stdout_write_structured`](Self::stdout_write_structured) instead of
    /// per-argument string fragments.
    ///
    /// When this returns `true`, `builtin_print` will convert each `Value` to a
    /// [`MontyObject`] and deliver all positional arguments in a single
    /// `stdout_write_structured` call, rather than calling `stdout_write` and
    /// `stdout_push` multiple times.
    #[must_use]
    pub fn wants_structured(&self) -> bool {
        match self {
            Self::Callback(cb) => cb.wants_structured(),
            _ => false,
        }
    }

    /// Called once per `print()` invocation with all positional arguments as
    /// [`AnnotatedObject`] values (each pairing a [`MontyObject`](crate::MontyObject)
    /// with optional provenance metadata), plus the resolved `sep` and `end`.
    ///
    /// JSON-serializable types (int, str, float, bool, None, list, dict, tuple)
    /// are passed as their native `MontyObject` variants. Non-serializable types
    /// (functions, iterators, etc.) are represented as `MontyObject::Repr` with
    /// a type name and their `repr()` string.
    pub fn stdout_write_structured(
        &mut self,
        objects: Vec<AnnotatedObject>,
        sep: &str,
        end: &str,
    ) -> Result<(), MontyException> {
        match self {
            Self::Callback(cb) => cb.stdout_write_structured(objects, sep, end),
            _ => unreachable!("stdout_write_structured called on non-structured writer"),
        }
    }
}

/// Trait for custom output handling from the `print()` builtin function.
///
/// Implement this trait and pass it via [`PrintWriter::Callback`] to capture
/// or redirect print output from sandboxed Python code.
pub trait PrintWriterCallback {
    /// Called once for each formatted argument passed to `print()`.
    ///
    /// This method is responsible for writing only the given argument's text, and must
    /// not add separators or a trailing newline. Separators (such as spaces) and the
    /// final terminator (such as a newline) are emitted via [`stdout_push`](Self::stdout_push).
    ///
    /// # Arguments
    /// * `output` - The formatted output string for a single argument (without
    ///   separators or trailing newline).
    fn stdout_write(&mut self, output: Cow<'_, str>) -> Result<(), MontyException>;

    /// Add a single character to stdout.
    ///
    /// Generally called to add spaces and newlines within print output.
    ///
    /// # Arguments
    /// * `end` - The character to print after the formatted output.
    fn stdout_push(&mut self, end: char) -> Result<(), MontyException>;

    /// Whether this callback wants structured output.
    ///
    /// When `true`, `builtin_print` calls [`stdout_write_structured`](Self::stdout_write_structured)
    /// once per `print()` invocation instead of calling `stdout_write`/`stdout_push` per argument.
    /// Defaults to `false` for backwards compatibility.
    fn wants_structured(&self) -> bool {
        false
    }

    /// Receives all positional arguments of a single `print()` call as
    /// [`AnnotatedObject`] values (each pairing a value with optional provenance
    /// metadata), along with the resolved `sep` and `end` strings.
    ///
    /// Only called when [`wants_structured`](Self::wants_structured) returns `true`.
    /// The default implementation falls back to `stdout_write`/`stdout_push`,
    /// formatting each object via its string representation.
    fn stdout_write_structured(
        &mut self,
        objects: Vec<AnnotatedObject>,
        sep: &str,
        end: &str,
    ) -> Result<(), MontyException> {
        let mut first = true;
        for obj in &objects {
            if first {
                first = false;
            } else {
                self.stdout_write(Cow::Borrowed(sep))?;
            }
            self.stdout_write(Cow::Owned(obj.value.to_string()))?;
        }
        self.stdout_write(Cow::Borrowed(end))?;
        Ok(())
    }
}
