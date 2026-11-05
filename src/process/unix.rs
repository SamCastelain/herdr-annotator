use super::{Capture, reap_after_kill};
use std::{
    io::{self, Read},
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Child, Command},
    time::Instant,
};

pub(super) trait Pipe: Read + AsRawFd {}
impl<T: Read + AsRawFd> Pipe for T {}

pub(super) struct Running {
    pub child: Child,
    complete: bool,
}

impl Running {
    pub fn spawn(command: &mut Command) -> io::Result<Self> {
        Ok(Self {
            child: command.process_group(0).spawn()?,
            complete: false,
        })
    }

    pub fn complete(&mut self) -> io::Result<()> {
        self.complete = true;
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        // Descendants may escape this process group. Nonblocking capture still
        // bounds local pipe ownership even when such a writer retains a pipe.
        // SAFETY: the group ID was established by process_group(0) at spawn.
        unsafe {
            libc::kill(-(self.child.id() as i32), libc::SIGKILL);
        }
        reap_after_kill(&mut self.child);
    }
}

pub(super) fn configure(reader: &impl Pipe) -> io::Result<()> {
    let fd = reader.as_raw_fd();
    // SAFETY: fd is borrowed from the owned pipe and remains open here.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(super) fn read(reader: &mut impl Pipe, buffer: &mut [u8]) -> io::Result<usize> {
    reader.read(buffer)
}

pub(super) fn wait<R: Pipe, S: Pipe>(
    stdout: &Capture<R>,
    stderr: &Capture<S>,
    deadline: Instant,
) -> io::Result<()> {
    fn pollfd<R: Pipe>(capture: &Capture<R>) -> libc::pollfd {
        libc::pollfd {
            fd: if capture.eof {
                -1
            } else {
                capture.reader.as_raw_fd()
            },
            events: libc::POLLIN,
            revents: 0,
        }
    }
    let mut fds = [pollfd(stdout), pollfd(stderr)];
    let millis = deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .clamp(1, 5) as i32;
    // SAFETY: fds is initialized and borrowed for the duration of poll.
    if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, millis) } < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}
