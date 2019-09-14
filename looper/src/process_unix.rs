use crate::{Call, Callback, Child, Core, ObjectId};
use libc;
use log::error;
use mio::{
    unix::{EventedFd, UnixReady},
    Evented, Poll, PollOpt, Ready, Token,
};
use signal_hook::iterator::Signals;
use stash::Stash;
use std::any::Any;
use std::collections::VecDeque;
use std::io;
use std::os::unix::io::{AsRawFd, RawFd};
use std::process;

pub fn new_core() -> Core {
    let signals = Signals::new(&[signal_hook::SIGCHLD]).unwrap();
    let mut core = Core {
        io_handlers: Stash::default(),
        objects: Stash::default(),
        poll: Poll::new().unwrap(),
        exit: false,
        process_handler: ProcessHandler {
            reapers: VecDeque::new(),
        },
    };
    core.register_reader(&signals, core.next_id(), reap_all);
    core.add(signals);
    core
}

pub fn register_reaper<F, T, S>(core: &mut Core, child: &Child<S>, object_id: ObjectId, f: F)
where
    F: 'static + FnMut(&mut T, &mut Core),
    T: Any,
{
    core.process_handler.reapers.push_back(Reaper {
        pid: child.child.id() as libc::pid_t,
        object_id,
        callback: Box::new(Callback::new(f)),
    });
}

struct Reaper {
    pid: libc::pid_t,
    object_id: ObjectId,
    callback: Box<Call>,
}

pub struct ProcessHandler {
    reapers: VecDeque<Reaper>,
}

fn reap_all(signals: &mut Signals, core: &mut Core) {
    // drain all pending signals, but we don't need to check which signal we got.
    for _ in signals.pending() {}
    for _ in 0..core.process_handler.reapers.len() {
        let mut r = core.process_handler.reapers.pop_front().unwrap();
        match reap(r.pid) {
            Ok(false) => core.process_handler.reapers.push_back(r),
            Ok(true) => {
                core.call_on_object(r.object_id, |obj, c| r.callback.make_call(obj, c));
            }
            Err(e) => error!("Failed to check if process has exited: {}", e),
        }
    }
}

fn reap(pid: libc::pid_t) -> io::Result<bool> {
    let mut status = 0;
    loop {
        match unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) } {
            0 => return Ok(false),
            n if n < 0 => {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                return Err(err);
            }
            n => {
                assert_eq!(n, pid);
                return Ok(true);
            }
        }
    }
}

#[derive(Debug)]
pub struct Fd<T>(T);

// FIXME: should be able to impl Into<Stdio> so that it can be passed to another Command

impl<T: io::Read> io::Read for Fd<T> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}

impl<T: io::Write> io::Write for Fd<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<T: AsRawFd> AsRawFd for Fd<T> {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

impl<T: AsRawFd> Evented for Fd<T> {
    fn register(
        &self,
        poll: &mio::Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.as_raw_fd()).register(poll, token, interest | UnixReady::hup(), opts)
    }

    fn reregister(
        &self,
        poll: &mio::Poll,
        token: Token,
        interest: Ready,
        opts: PollOpt,
    ) -> io::Result<()> {
        EventedFd(&self.as_raw_fd()).reregister(poll, token, interest | UnixReady::hup(), opts)
    }

    fn deregister(&self, poll: &mio::Poll) -> io::Result<()> {
        EventedFd(&self.as_raw_fd()).deregister(poll)
    }
}

pub type Stdin = Fd<process::ChildStdin>;
pub type Stdout = Fd<process::ChildStdout>;
pub type Stderr = Fd<process::ChildStderr>;

pub fn new_child(mut child: process::Child) -> io::Result<Child<Stdin>> {
    let stdin = make_nonblocking(child.stdin.take().unwrap())?;
    let stdout = make_nonblocking(child.stdout.take().unwrap())?;
    let stderr = make_nonblocking(child.stderr.take().unwrap())?;
    Ok(Child {
        child,
        stdin,
        stdout,
        stderr,
    })
}

// Set the fd to nonblocking before we pass it to the event loop
fn make_nonblocking<T: AsRawFd>(io: T) -> io::Result<Fd<T>> {
    let fd = io.as_raw_fd();
    unsafe {
        let r = libc::fcntl(fd, libc::F_GETFL);
        if r == -1 {
            return Err(io::Error::last_os_error());
        }
        let r = libc::fcntl(fd, libc::F_SETFL, r | libc::O_NONBLOCK);
        if r == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(Fd(io))
}
