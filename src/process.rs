//! Bounded process lifetime, capture, and diagnostics. Exit meanings belong
//! to command adapters. No timeout here proves cancellation of remote delivery.
use crate::Result;
use std::{
    cell::Cell,
    io,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
use unix as backend;
#[cfg(windows)]
use windows as backend;

pub(crate) const MAX_OUTPUT: usize = 512 * 1024;
const BUDGET: Duration = Duration::from_secs(25);
thread_local! { static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) }; }

// Scoped per Engine request, including pagination and delivery. Nested calls
// inherit the earlier deadline. The guard restores it on errors and unwinding.
pub(crate) fn within_budget<T>(work: impl FnOnce() -> T) -> T {
    with_deadline(Instant::now() + BUDGET, work)
}

pub(crate) fn within_generation_budget<T>(work: impl FnOnce() -> T) -> T {
    with_deadline(Instant::now() + Duration::from_secs(120), work)
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

fn reap_after_kill(child: &mut Child) {
    let _ = child.kill();
    let until = Instant::now() + Duration::from_millis(50);
    while Instant::now() < until {
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => std::thread::sleep(Duration::from_millis(1)),
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

impl<R: backend::Pipe> Capture<R> {
    fn new(reader: R, limit: usize) -> Result<Self> {
        backend::configure(&reader).map_err(|e| format!("Cannot configure capture pipe: {e}"))?;
        Ok(Self {
            reader,
            bytes: Vec::new(),
            limit,
            truncated: false,
            eof: false,
        })
    }

    fn drain(&mut self) -> io::Result<()> {
        if self.eof {
            return Ok(());
        }
        // Fairness: continuous stdout must not starve stderr or deadline checks.
        for _ in 0..8 {
            let mut buf = [0; 8192];
            match backend::read(&mut self.reader, &mut buf) {
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
}

pub(crate) fn run(command: &mut Command, limit: usize) -> Result<Output> {
    let deadline = DEADLINE.get().unwrap_or_else(|| Instant::now() + BUDGET);
    if Instant::now() >= deadline {
        return Err("Command timed out: shared 25-second subprocess budget exhausted".into());
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut running = backend::Running::spawn(command).map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            format!("Cannot start command (not found): {e}")
        } else {
            format!("Cannot start command: {e}")
        }
    })?;
    let mut stdout = Capture::new(running.child.stdout.take().ok_or("Missing stdout")?, limit)?;
    let mut stderr = Capture::new(running.child.stderr.take().ok_or("Missing stderr")?, 8192)?;
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
            match running.child.try_wait() {
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
        if let Err(e) = backend::wait(&stdout, &stderr, drain_deadline) {
            break Err(format!("Cannot poll command output: {e}"));
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
    running
        .complete()
        .map_err(|e| format!("Cannot release completed command: {e}"))?;
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
    use std::sync::{Mutex, MutexGuard};

    // These tests deliberately use sub-second deadlines. Concurrent Python
    // startup (especially Windows executable scanning) can exhaust the budget
    // before the fixture runs. Acquire this before starting any test deadline;
    // production concurrency and deadlines are unchanged.
    static SUBPROCESS_FIXTURE: Mutex<()> = Mutex::new(());

    pub(super) fn subprocess_fixture() -> MutexGuard<'static, ()> {
        SUBPROCESS_FIXTURE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(super) fn python(source: &str) -> Command {
        #[cfg(windows)]
        let default = "python";
        #[cfg(unix)]
        let default = "python3";
        let executable =
            std::env::var_os("HERDR_ANNOTATOR_TEST_PYTHON").unwrap_or_else(|| default.into());
        let mut command = Command::new(executable);
        command.args(["-c", source]);
        command
    }

    fn short<T>(work: impl FnOnce() -> T) -> T {
        // Native Windows Python startup can exceed 200 ms on CI machines.
        with_deadline(Instant::now() + Duration::from_millis(800), work)
    }

    #[test]
    fn exits_are_facts_and_diagnostics_are_retained() {
        let _fixture = subprocess_fixture();
        let out = run(
            &mut python(
                "import sys; print('reply'); print('denied', file=sys.stderr); sys.exit(7)",
            ),
            100,
        )
        .unwrap();
        assert_eq!(out.code, 7);
        assert_eq!(out.text.replace("\r\n", "\n"), "reply\n");
        assert!(out.failure("test").contains("denied"));
        assert!(out.failure("test").contains("reply"));
    }

    #[test]
    fn timeout_preserves_diagnostics_and_bounds_cleanup() {
        let _fixture = subprocess_fixture();
        let start = Instant::now();
        let error = short(|| run(&mut python("import sys,time; print('still working',file=sys.stderr,flush=True); time.sleep(10)"), 100)).err().unwrap();
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("still working"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn child_exit_does_not_wait_forever_for_inherited_pipes() {
        let _fixture = subprocess_fixture();
        let start = Instant::now();
        let error = run(
            &mut python(
                "import os,subprocess,sys; subprocess.Popen([sys.executable,'-c','import time; time.sleep(10)'], stdout=sys.stdout, stderr=sys.stderr); os._exit(0)",
            ),
            100,
        )
        .err()
        .unwrap();
        assert!(error.contains("output did not close"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn escaped_pipe_writer_cannot_extend_capture_lifetime() {
        let _fixture = subprocess_fixture();
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
    fn closed_pipes_do_not_hide_a_hanging_child() {
        let _fixture = subprocess_fixture();
        let error = short(|| {
            run(
                &mut python("import os,time; os.close(1); os.close(2); time.sleep(10)"),
                100,
            )
        })
        .err()
        .unwrap();
        assert!(error.contains("timed out"));
    }

    #[cfg(unix)]
    #[test]
    fn signals_are_not_success() {
        let _fixture = subprocess_fixture();
        let out = run(
            &mut python("import os,signal; os.kill(os.getpid(),signal.SIGTERM)"),
            100,
        )
        .unwrap();
        assert_eq!(out.code, -1);
    }

    #[test]
    fn nonexistent_command_is_not_success() {
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
        let _fixture = subprocess_fixture();
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
        let _fixture = subprocess_fixture();
        let error = short(|| run(&mut python("import os\nos.write(2,b'flood diagnostic\\n')\nwhile True: os.write(1,b'x'*8192)"), 1024)).err().unwrap();
        assert!(error.contains("timed out"));
        assert!(error.contains("flood diagnostic"));
    }

    #[test]
    fn output_utf8_is_strict_but_stderr_is_lossy() {
        let _fixture = subprocess_fixture();
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
        let _fixture = subprocess_fixture();
        with_deadline(Instant::now() + Duration::from_millis(2000), || {
            run(&mut python("import time; time.sleep(1.1)"), 100).unwrap();
            let error = run(&mut python("import time; time.sleep(1.1)"), 100)
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

    #[test]
    fn concurrent_commands_keep_output_and_deadlines_independent() {
        let _fixture = subprocess_fixture();
        // Deliberately run two commands concurrently inside one fixture slot.
        // The timeout test needs no startup marker; the other command retains
        // its normal budget and must not inherit the first thread's deadline.
        std::thread::scope(|scope| {
            let timeout =
                scope.spawn(|| short(|| run(&mut python("import time; time.sleep(10)"), 100)));
            let success = scope.spawn(|| run(&mut python("print('independent')"), 100).unwrap());
            assert!(timeout.join().unwrap().err().unwrap().contains("timed out"));
            let output = success.join().unwrap();
            assert_eq!(output.code, 0);
            assert_eq!(output.text.trim(), "independent");
        });
    }

    #[test]
    fn literal_arguments_survive_without_shell_interpretation() {
        let _fixture = subprocess_fixture();
        let argument = "spaces (parentheses) 'quotes' \"double\" & | %PATH% ! ^ 中文";
        let mut command = python("import os,sys; os.write(1,sys.argv[1].encode('utf-8'))");
        command.arg(argument);
        assert_eq!(run(&mut command, 1024).unwrap().text, argument);
    }

    #[test]
    fn successful_commands_may_leave_background_work_without_inherited_pipes() {
        let _fixture = subprocess_fixture();
        let temp = tempfile::tempdir().unwrap();
        let marker = temp.path().join("background-finished");
        let background = format!(
            "import pathlib,time; time.sleep(.4); pathlib.Path({:?}).write_text('finished')",
            marker.to_str().unwrap()
        );
        let source = format!(
            "import os,subprocess,sys; subprocess.Popen([sys.executable,'-c',{}], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL); os._exit(0)",
            serde_json::to_string(&background).unwrap()
        );
        assert_eq!(run(&mut python(&source), 100).unwrap().code, 0);
        let until = Instant::now() + Duration::from_secs(5);
        while !marker.exists() {
            assert!(
                Instant::now() < until,
                "Successful command's background work was killed"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn nested_budgets_never_extend_the_earlier_deadline_and_restore_after_unwinding() {
        let deadline = Instant::now() + Duration::from_secs(1);
        let result = std::panic::catch_unwind(|| {
            with_deadline(deadline, || {
                within_generation_budget(|| assert_eq!(DEADLINE.get(), Some(deadline)));
                assert_eq!(DEADLINE.get(), Some(deadline));
                panic!("exercise guard unwinding");
            });
        });
        assert!(result.is_err());
        assert_eq!(DEADLINE.get(), None);
    }

    #[cfg(windows)]
    #[test]
    fn job_terminates_descendants_on_timeout_and_inherited_pipe_failure() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0,
            System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject},
        };

        let _fixture = subprocess_fixture();
        for parent_exits in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let pidfile = temp.path().join("descendant.pid");
            let ready = temp.path().join("observer-ready");
            let source = format!(
                "import os,pathlib,subprocess,sys,time\np = subprocess.Popen([sys.executable,'-c','import time; time.sleep(30)'], stdout=sys.stdout, stderr=sys.stderr)\npathlib.Path({:?}).write_text(str(p.pid))\nwhile not pathlib.Path({:?}).exists(): time.sleep(.001)\n{}",
                pidfile.to_str().unwrap(),
                ready.to_str().unwrap(),
                if parent_exits {
                    "os._exit(0)"
                } else {
                    "time.sleep(30)"
                }
            );
            // Inspect the descendant while the job is still live, and retain a
            // handle so PID reuse cannot make the termination assertion pass.
            let observer = std::thread::spawn(move || {
                let until = Instant::now() + Duration::from_secs(5);
                loop {
                    if let Ok(text) = std::fs::read_to_string(&pidfile)
                        && let Ok(pid) = text.parse::<u32>()
                    {
                        // SAFETY: OpenProcess validates the PID and returns
                        // a handle restricted to waiting for termination.
                        let handle = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
                        assert!(
                            !handle.is_null(),
                            "Cannot inspect descendant: {}",
                            io::Error::last_os_error()
                        );
                        // SAFETY: OpenProcess returned a newly owned handle.
                        let process = unsafe { OwnedHandle::from_raw_handle(handle) };
                        std::fs::write(&ready, "ready").unwrap();
                        return process;
                    }
                    assert!(Instant::now() < until, "Descendant did not start");
                    std::thread::sleep(Duration::from_millis(1));
                }
            });
            let start = Instant::now();
            let error = with_deadline(Instant::now() + Duration::from_secs(3), || {
                run(&mut python(&source), 100)
            })
            .err()
            .unwrap();
            assert!(
                error.contains(if parent_exits {
                    "output did not close"
                } else {
                    "timed out"
                }),
                "{error}"
            );
            assert!(start.elapsed() < Duration::from_secs(5));
            let descendant = observer.join().unwrap();
            // SAFETY: the observer owns a valid process handle with SYNCHRONIZE.
            assert_eq!(
                unsafe { WaitForSingleObject(descendant.as_raw_handle(), 1000) },
                WAIT_OBJECT_0,
                "Descendant survived cleanup"
            );
        }
    }
}
