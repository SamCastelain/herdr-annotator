//! Unix process lifetime, bounded capture, and diagnostics. Exit meanings belong
//! to command adapters. No timeout here proves cancellation of remote delivery.
use crate::Result;
use std::{
    cell::Cell,
    io::{self, Read},
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

pub(crate) const MAX_OUTPUT: usize = 512 * 1024;
const BUDGET: Duration = Duration::from_secs(25);
thread_local! { static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) }; }

// Scoped per Engine request, including pagination and delivery. Nested calls
// inherit the earlier deadline. The guard restores it on errors and unwinding.
pub(crate) fn within_budget<T>(work: impl FnOnce() -> T) -> T {
    with_deadline(Instant::now() + BUDGET, work)
}

fn with_deadline<T>(deadline: Instant, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Instant>);
    impl Drop for Restore {
        fn drop(&mut self) {
            DEADLINE.set(self.0);
        }
    }
    let previous = DEADLINE.get();
    let _restore = Restore(previous);
    DEADLINE.set(Some(previous.map_or(deadline, |old| old.min(deadline))));
    work()
}

pub(crate) struct Output {
    pub code: i32,
    pub text: String,
    pub truncated: bool,
    pub stderr: String,
    pub stderr_truncated: bool,
}

impl Output {
    pub fn failure(&self, command: &str) -> String {
        let diagnostic: String = self.text.chars().take(8192).collect();
        let omitted = diagnostic.len() < self.text.len() || self.truncated;
        format!(
            "{command} failed ({}): {}{}{}",
            self.code,
            self.stderr.trim(),
            if self.stderr_truncated {
                " [stderr truncated]"
            } else {
                ""
            },
            if diagnostic.trim().is_empty() {
                String::new()
            } else {
                format!(
                    "\n{}{}",
                    diagnostic.trim(),
                    if omitted { " [stdout truncated]" } else { "" }
                )
            }
        )
    }
}

struct Running(Child, bool);
impl Drop for Running {
    fn drop(&mut self) {
        if self.1 {
            return;
        }
        // Each command starts its own process group. Descendants may escape it;
        // dropping our nonblocking read ends still bounds local pipe ownership.
        // SAFETY: kill receives the process group ID created by process_group(0).
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.kill();
        let until = Instant::now() + Duration::from_millis(50);
        while Instant::now() < until {
            match self.0.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(1)),
            }
        }
    }
}

struct Capture<R> {
    reader: R,
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
    eof: bool,
}

impl<R: Read + AsRawFd> Capture<R> {
    fn new(reader: R, limit: usize) -> Result<Self> {
        let fd = reader.as_raw_fd();
        // SAFETY: fd is borrowed from the owned pipe and remains open here.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(format!(
                "Cannot configure capture pipe: {}",
                io::Error::last_os_error()
            ));
        }
        Ok(Self {
            reader,
            bytes: Vec::new(),
            limit,
            truncated: false,
            eof: false,
        })
    }

    fn drain(&mut self) -> io::Result<()> {
        // Fairness: continuous stdout must not starve stderr or deadline checks.
        for _ in 0..8 {
            let mut buf = [0; 8192];
            match self.reader.read(&mut buf) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(n) => {
                    let keep = n.min(self.limit.saturating_sub(self.bytes.len()));
                    self.bytes.extend_from_slice(&buf[..keep]);
                    self.truncated |= keep != n;
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    fn pollfd(&self) -> libc::pollfd {
        libc::pollfd {
            fd: if self.eof {
                -1
            } else {
                self.reader.as_raw_fd()
            },
            events: libc::POLLIN,
            revents: 0,
        }
    }
}

pub(crate) fn run(command: &mut Command, limit: usize) -> Result<Output> {
    let deadline = DEADLINE.get().unwrap_or_else(|| Instant::now() + BUDGET);
    if Instant::now() >= deadline {
        return Err("Command timed out: shared 25-second subprocess budget exhausted".into());
    }
    let mut running = Running(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    format!("Cannot start command (not found): {e}")
                } else {
                    format!("Cannot start command: {e}")
                }
            })?,
        false,
    );
    let mut stdout = Capture::new(running.0.stdout.take().ok_or("Missing stdout")?, limit)?;
    let mut stderr = Capture::new(running.0.stderr.take().ok_or("Missing stderr")?, 8192)?;
    let mut status = None;
    let mut drain_deadline = deadline;
    let result = loop {
        if Instant::now() >= drain_deadline {
            break Err(if status.is_some() {
                "Command output did not close before the capture deadline".into()
            } else {
                "Command timed out: shared 25-second subprocess budget exhausted".into()
            });
        }
        if let Err(e) = stdout.drain().and_then(|()| stderr.drain()) {
            break Err(format!("Cannot read command output: {e}"));
        }
        if status.is_none() {
            match running.0.try_wait() {
                Ok(Some(exit)) => {
                    status = Some(exit);
                    drain_deadline = deadline.min(Instant::now() + Duration::from_millis(250));
                }
                Ok(None) => (),
                Err(e) => break Err(format!("Cannot wait for command: {e}")),
            }
        }
        if status.is_some() && stdout.eof && stderr.eof {
            break Ok(());
        }
        let mut fds = [stdout.pollfd(), stderr.pollfd()];
        let millis = drain_deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .clamp(1, 5) as i32;
        // SAFETY: fds is an initialized array, borrowed for the duration of poll.
        let polled = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, millis) };
        if polled < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            break Err(format!(
                "Cannot poll command output: {}",
                io::Error::last_os_error()
            ));
        }
    };
    let diagnostics = String::from_utf8_lossy(&stderr.bytes).into_owned();
    result.map_err(|e: String| {
        format!(
            "{e}: {}{}",
            diagnostics.trim(),
            if stderr.truncated {
                " [stderr truncated]"
            } else {
                ""
            }
        )
    })?;
    // Child has been reaped and both pipes reached EOF. No cancellation needed.
    running.1 = true;
    let text = String::from_utf8(stdout.bytes)
        .map_err(|_| format!("Non-UTF-8 command output: {}", diagnostics.trim()))?;
    Ok(Output {
        code: status.and_then(|s| s.code()).unwrap_or(-1),
        text,
        truncated: stdout.truncated,
        stderr: diagnostics,
        stderr_truncated: stderr.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn python(source: &str) -> Command {
        let mut command = Command::new("python3");
        command.args(["-c", source]);
        command
    }

    fn short<T>(work: impl FnOnce() -> T) -> T {
        with_deadline(Instant::now() + Duration::from_millis(200), work)
    }

    #[test]
    fn exits_are_facts_and_diagnostics_are_retained() {
        let out = run(
            &mut python(
                "import sys; print('reply'); print('denied', file=sys.stderr); sys.exit(7)",
            ),
            100,
        )
        .unwrap();
        assert_eq!(out.code, 7);
        assert_eq!(out.text, "reply\n");
        assert!(out.failure("test").contains("denied"));
        assert!(out.failure("test").contains("reply"));
    }

    #[test]
    fn timeout_preserves_diagnostics_and_bounds_cleanup() {
        let start = Instant::now();
        let error = short(|| run(&mut python("import sys,time; print('still working',file=sys.stderr,flush=True); time.sleep(10)"), 100)).err().unwrap();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("still working"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn child_exit_does_not_wait_forever_for_inherited_pipes() {
        let start = Instant::now();
        let error = run(
            &mut python(
                "import os,time; pid=os.fork(); time.sleep(10) if pid==0 else None; os._exit(0)",
            ),
            100,
        )
        .err()
        .unwrap();
        assert!(error.contains("output did not close"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn escaped_pipe_writer_cannot_extend_capture_lifetime() {
        let temp = tempfile::tempdir().unwrap();
        let pidfile = temp.path().join("writer.pid");
        let source = format!(
            "import os,time\npid=os.fork()\nif pid==0:\n os.setsid()\n open({:?},'w').write(str(os.getpid()))\n time.sleep(10)\nelse:\n os._exit(0)",
            pidfile.to_str().unwrap()
        );
        let start = Instant::now();
        let error = run(&mut python(&source), 100).err().unwrap();
        let pid: i32 = std::fs::read_to_string(pidfile).unwrap().parse().unwrap();
        // SAFETY: pid identifies only the escaped writer created by this test.
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
        assert!(error.contains("output did not close"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn closed_pipes_do_not_hide_a_hanging_child_and_signals_are_not_success() {
        let error = short(|| {
            run(
                &mut python("import os,time; os.close(1); os.close(2); time.sleep(10)"),
                100,
            )
        })
        .err()
        .unwrap();
        assert!(error.contains("timed out"));
        let out = run(
            &mut python("import os,signal; os.kill(os.getpid(),signal.SIGTERM)"),
            100,
        )
        .unwrap();
        assert_eq!(out.code, -1);
        assert!(
            run(
                &mut Command::new("herdr-annotator-nonexistent-test-command"),
                100
            )
            .err()
            .unwrap()
            .contains("Cannot start command")
        );
    }

    #[test]
    fn finite_floods_drain_both_pipes_beyond_retention_limits() {
        let out = run(
            &mut python(
                "import os; [(os.write(1,b'x'*8192),os.write(2,b'e'*8192)) for _ in range(128)]",
            ),
            1024,
        )
        .unwrap();
        assert_eq!(out.code, 0);
        assert_eq!(out.text.len(), 1024);
        assert_eq!(out.stderr.len(), 8192);
        assert!(out.truncated && out.stderr_truncated);
    }

    #[test]
    fn endless_flood_cannot_starve_deadline_or_stderr() {
        let error = short(|| run(&mut python("import os\nos.write(2,b'flood diagnostic\\n')\nwhile True: os.write(1,b'x'*8192)"), 1024)).err().unwrap();
        assert!(error.contains("timed out"));
        assert!(error.contains("flood diagnostic"));
    }

    #[test]
    fn output_utf8_is_strict_but_stderr_is_lossy() {
        assert!(
            run(&mut python("import os; os.write(1,b'\\xff')"), 100)
                .err()
                .unwrap()
                .contains("Non-UTF-8")
        );
        let out = run(&mut python("import os; os.write(2,b'\\xff')"), 100).unwrap();
        assert_eq!(out.stderr, "�");
    }

    #[test]
    fn commands_share_the_request_deadline_and_expiry_prevents_spawn() {
        with_deadline(Instant::now() + Duration::from_millis(800), || {
            run(&mut python("import time; time.sleep(.45)"), 100).unwrap();
            let error = run(&mut python("import time; time.sleep(.45)"), 100)
                .err()
                .unwrap();
            assert!(error.contains("timed out"));
            let error = run(&mut Command::new("must-not-start"), 100).err().unwrap();
            assert!(error.contains("budget exhausted"));
        });
        // Scope was restored, including after an exhausted request.
        assert_eq!(
            run(&mut python("print('next request')"), 100).unwrap().code,
            0
        );
    }
}
