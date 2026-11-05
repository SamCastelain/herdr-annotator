//! Job containment covers directly created processes and their descendants.
//! App Execution Aliases may activate a runtime through an external broker;
//! callers must use real executable paths when child-tree cleanup is required.
use super::{Capture, reap_after_kill};
use std::{
    io::{self, Read},
    mem::{size_of, zeroed},
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::{Child, Command},
    ptr::{null, null_mut},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_BROKEN_PIPE, ERROR_NO_MORE_FILES, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Pipes::PeekNamedPipe,
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};

pub(super) trait Pipe: Read + AsRawHandle {}
impl<T: Read + AsRawHandle> Pipe for T {}

struct Job(OwnedHandle);

impl Job {
    fn new() -> io::Result<Self> {
        // SAFETY: null security/name arguments request an unnamed, non-inherited job.
        let handle = unsafe { CreateJobObjectW(null(), null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: CreateJobObjectW returned a new owned handle.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(handle) });
        job.kill_on_close(true)?;
        Ok(job)
    }

    fn kill_on_close(&self, enabled: bool) -> io::Result<()> {
        // SAFETY: this Windows POD structure accepts zero initialization.
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
        if enabled {
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        }
        // SAFETY: job handle is live and limits has the required layout/size.
        if unsafe {
            SetInformationJobObject(
                self.0.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

pub(super) struct Running {
    pub child: Child,
    job: Job,
    complete: bool,
}

impl Running {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        let job = Job::new().map_err(|e| {
            io::Error::new(e.kind(), format!("Cannot create/configure child job: {e}"))
        })?;
        // Stable std exposes creation flags but not the primary thread handle
        // or process attribute lists. Start suspended, assign the job, then
        // locate/resume its only thread through documented ToolHelp APIs. No
        // child code can run or create descendants before job assignment.
        let child = command
            .creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW)
            .spawn()?;
        let running = Self {
            child,
            job,
            complete: false,
        };
        // SAFETY: both handles are live and owned by running. If assignment or
        // resumption fails, its Drop kills/reaps even an unassigned child.
        if unsafe {
            AssignProcessToJobObject(running.job.0.as_raw_handle(), running.child.as_raw_handle())
        } == 0
        {
            let error = io::Error::last_os_error();
            return Err(io::Error::new(
                error.kind(),
                format!("Cannot assign suspended child to job: {error}"),
            ));
        }
        resume_primary_thread(running.child.id())
            .map_err(|e| io::Error::new(e.kind(), format!("Cannot resume suspended child: {e}")))?;
        Ok(running)
    }

    pub fn complete(&mut self) -> io::Result<()> {
        // Match Unix success: an intentional background descendant which has
        // closed its output pipes may survive a successful command. Errors and
        // unwinding keep kill-on-close enabled and terminate the whole job.
        self.job.kill_on_close(false)?;
        self.complete = true;
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.complete {
            // Terminate the job first to stop descendants, then bound reaping
            // of the direct child. OwnedHandle closes all other owned handles.
            // Explicit termination avoids relying on struct field drop order.
            // SAFETY: this job handle is live and contains only our child tree.
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(
                    self.job.0.as_raw_handle(),
                    1,
                );
            }
            reap_after_kill(&mut self.child);
        }
    }
}

fn resume_primary_thread(pid: u32) -> io::Result<()> {
    // SAFETY: snapshot flags and process ID are valid documented inputs.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: snapshot is a newly owned handle, not INVALID_HANDLE_VALUE.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(snapshot) };
    // SAFETY: THREADENTRY32 is POD; dwSize is set before passing to ToolHelp.
    let mut entry: THREADENTRY32 = unsafe { zeroed() };
    entry.dwSize = size_of::<THREADENTRY32>() as u32;
    // SAFETY: snapshot and entry remain live for the enumeration.
    let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) };
    let mut primary = None;
    while found != 0 {
        if entry.th32OwnerProcessID == pid && primary.replace(entry.th32ThreadID).is_some() {
            return Err(io::Error::other(
                "Suspended child has multiple threads; refusing ambiguous resumption",
            ));
        }
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        // SAFETY: snapshot and entry remain live for the enumeration.
        found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) };
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
        return Err(error);
    }
    let primary = primary.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "Suspended child's primary thread was not found",
        )
    })?;
    // SAFETY: the suspended child cannot exit/recycle its primary thread
    // normally before we resume it. Fail closed on API errors.
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, primary) };
    if thread.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenThread returned a new owned handle.
    let thread = unsafe { OwnedHandle::from_raw_handle(thread) };
    // SAFETY: thread is live and has THREAD_SUSPEND_RESUME access.
    if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn configure(_reader: &impl Pipe) -> io::Result<()> {
    Ok(())
}

pub(super) fn read(reader: &mut impl Pipe, buffer: &mut [u8]) -> io::Result<usize> {
    let mut available = 0;
    // SAFETY: the borrowed pipe handle stays live. Only available is written;
    // null buffer/other outputs request an availability query, not a read.
    if unsafe {
        PeekNamedPipe(
            reader.as_raw_handle() as HANDLE,
            null_mut(),
            0,
            null_mut(),
            &mut available,
            null_mut(),
        )
    } == 0
    {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) {
            return Ok(0);
        }
        return Err(error);
    }
    if available == 0 {
        return Err(io::ErrorKind::WouldBlock.into());
    }
    // Capture owns the sole reader. No other thread can consume the bytes
    // between PeekNamedPipe and this read, so it cannot wait for more input.
    let count = buffer.len().min(available as usize);
    reader.read(&mut buffer[..count])
}

pub(super) fn wait<R: Pipe, S: Pipe>(
    _stdout: &Capture<R>,
    _stderr: &Capture<S>,
    deadline: Instant,
) -> io::Result<()> {
    std::thread::sleep(
        deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(5)),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::windows::io::AsHandle, process::Stdio};
    use windows_sys::Win32::{
        Foundation::WAIT_OBJECT_0,
        System::{JobObjects::IsProcessInJob, Threading::WaitForSingleObject},
    };

    #[test]
    fn spawned_child_belongs_to_our_job_and_unwinding_terminates_it() {
        let _fixture = super::super::tests::subprocess_fixture();
        let mut command = super::super::tests::python("import time; time.sleep(30)");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let running = Running::spawn(&mut command).unwrap();
        let process = running.child.as_handle().try_clone_to_owned().unwrap();
        let mut in_job = 0;
        // SAFETY: both owned handles are live; in_job is a writable BOOL.
        assert_ne!(
            unsafe {
                IsProcessInJob(
                    process.as_raw_handle(),
                    running.job.0.as_raw_handle(),
                    &mut in_job,
                )
            },
            0
        );
        assert_ne!(in_job, 0);
        let result = std::panic::catch_unwind(move || {
            let _running = running;
            panic!("exercise subprocess guard unwinding");
        });
        assert!(result.is_err());
        // SAFETY: process is a live owned handle with SYNCHRONIZE access.
        assert_eq!(
            unsafe { WaitForSingleObject(process.as_raw_handle(), 1000) },
            WAIT_OBJECT_0
        );
    }
}
