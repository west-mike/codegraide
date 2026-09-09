//! One fallible stdout sink per invocation; no process-global error state.
use std::{
    fmt,
    io::{self, Write},
    process::ExitCode,
};

pub(crate) struct Output<W> {
    writer: W,
    error: Option<io::Error>,
}
impl<W: Write> Output<W> {
    pub(crate) fn new(writer: W) -> Self {
        Self {
            writer,
            error: None,
        }
    }
    pub(crate) fn write(&mut self, args: fmt::Arguments<'_>) {
        if self.error.is_none() {
            self.error = self.writer.write_fmt(args).err();
        }
    }
    pub(crate) fn line(&mut self, args: fmt::Arguments<'_>) {
        self.write(args);
        self.write(format_args!("\n"));
    }
    pub(crate) fn finish(mut self, status: ExitCode) -> ExitCode {
        if self.error.is_none() {
            self.error = self.writer.flush().err();
        }
        match self.error {
            None => status,
            Some(error) if error.kind() == io::ErrorKind::BrokenPipe => status,
            Some(error) => {
                let _ = writeln!(
                    io::stderr().lock(),
                    "error: could not write output: {error}"
                );
                ExitCode::FAILURE
            }
        }
    }
}
pub(crate) type Terminal<'a> = Output<io::StdoutLock<'a>>;

#[cfg(test)]
mod tests {
    use super::*;
    struct Failed(io::ErrorKind);
    impl Write for Failed {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(self.0))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn distinguishes_closed_consumer_from_other_write_failures() {
        for (kind, expected) in [
            (io::ErrorKind::BrokenPipe, ExitCode::SUCCESS),
            (io::ErrorKind::PermissionDenied, ExitCode::FAILURE),
        ] {
            let mut sink = Output::new(Failed(kind));
            sink.line(format_args!("data"));
            assert_eq!(sink.finish(ExitCode::SUCCESS), expected);
        }
    }
    #[test]
    fn closed_pipe_preserves_an_analysis_gate_failure() {
        let mut sink = Output::new(Failed(io::ErrorKind::BrokenPipe));
        sink.line(format_args!("gate finding"));
        assert_eq!(sink.finish(ExitCode::from(2)), ExitCode::from(2));
    }
}
