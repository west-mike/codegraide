//! Bounded command execution shared by Git ingestion and environment probes.
//!
//! Temporary output files avoid pipe-reader threads whose lifetime can outlast
//! the child. Output collection stops when the direct child exits. On Unix a
//! private process group also lets the owner clean up inherited descendants.
use std::{
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

struct Running {
    child: Child,
    cleaned: bool,
}
impl Running {
    fn cleanup(&mut self) -> io::Result<()> {
        let mut failure = None;
        #[cfg(unix)]
        {
            use nix::{
                errno::Errno,
                sys::signal::{Signal, killpg},
                unistd::Pid,
            };
            if let Err(error) = killpg(Pid::from_raw(self.child.id() as i32), Signal::SIGKILL) {
                if error != Errno::ESRCH {
                    failure = Some(io::Error::from_raw_os_error(error as i32));
                }
            }
        }
        if self.child.try_wait()?.is_none() {
            if let Err(error) = self.child.kill() {
                if self.child.try_wait()?.is_none() {
                    failure.get_or_insert(error);
                }
            }
        }
        self.child.wait()?;
        self.cleaned = true;
        failure.map_or(Ok(()), Err)
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        // The normal path reports cleanup failures. Drop is the panic fallback.
        if !self.cleaned {
            let _ = self.cleanup();
        }
    }
}

fn read_output(mut file: File, limit: usize) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::other("command output exceeded its byte limit"));
    }
    Ok(bytes)
}

/// Runs until `deadline`, retaining at most `limit` bytes from each output.
/// Output files are polled every 10 ms; a fast writer can transiently use more
/// disk space, but retained memory and output remain bounded. No shell is used.
/// On non-Unix systems cleanup terminates the direct child only; inherited
/// output handles cannot delay this function because collection uses files.
pub fn run(command: &mut Command, deadline: Instant, limit: usize) -> io::Result<Output> {
    let stdout = tempfile::tempfile()?;
    let stderr = tempfile::tempfile()?;
    command
        .stdout(Stdio::from(stdout.try_clone()?))
        .stderr(Stdio::from(stderr.try_clone()?));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "command deadline exceeded",
        ));
    }
    let mut child = Running {
        child: command.spawn()?,
        cleaned: false,
    };
    let status = (|| {
        loop {
            if stdout.metadata()?.len() > limit as u64 || stderr.metadata()?.len() > limit as u64 {
                return Err(io::Error::other("command output exceeded its byte limit"));
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "command deadline exceeded",
                ));
            }
            if let Some(status) = child.child.try_wait()? {
                return Ok(status);
            }
            std::thread::sleep(
                Duration::from_millis(10).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    })();
    let status = match (status, child.cleanup()) {
        (Ok(status), Ok(())) => status,
        (Err(error), Ok(())) | (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(cleanup)) => {
            return Err(io::Error::new(
                error.kind(),
                format!("{error}; process cleanup failed: {cleanup}"),
            ));
        }
    };
    Ok(Output {
        status,
        stdout: read_output(stdout, limit)?,
        stderr: read_output(stderr, limit)?,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    fn shell(script: &str, duration: Duration, limit: usize) -> io::Result<Output> {
        run(
            Command::new("sh").args(["-c", script]).stdin(Stdio::null()),
            Instant::now() + duration,
            limit,
        )
    }
    #[test]
    fn inherited_output_does_not_extend_the_operation() {
        let start = Instant::now();
        let output = shell(
            "sleep 2 & printf complete",
            Duration::from_millis(500),
            1024,
        )
        .unwrap();
        assert_eq!(output.stdout, b"complete");
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn deadline_and_output_limits_are_enforced() {
        assert_eq!(
            shell("sleep 2", Duration::from_millis(30), 1024)
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(shell("printf too-long", Duration::from_secs(1), 3).is_err());
    }
}
