use log::trace;
use mio::{Evented, Events as MioEvents, Poll, PollOpt, Ready, Token};
use stash::Stash;
use std::any::Any;
use std::borrow::{Borrow, BorrowMut};
use std::io;
use std::marker::PhantomData;
use std::process::{Child as ProcessChild, Command, Stdio};

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId(usize);

impl From<usize> for ObjectId {
    fn from(idx: usize) -> Self {
        ObjectId(idx)
    }
}
impl Into<usize> for ObjectId {
    fn into(self) -> usize {
        self.0
    }
}

trait Call {
    fn make_call(&mut self, _: &mut Any, _: &mut Core);
}

struct IoHandler {
    object_id: ObjectId,
    read_fn: Option<Box<Call>>,
    write_fn: Option<Box<Call>>,
}

struct Callback<F, T> {
    f: F,
    _marker: PhantomData<T>,
}

impl<F, T> Callback<F, T> {
    fn new(f: F) -> Callback<F, T> {
        Callback {
            f,
            _marker: PhantomData,
        }
    }
}

impl<F, T> Call for Callback<F, T>
where
    F: FnMut(&mut T, &mut Core),
    T: Any,
{
    fn make_call(&mut self, object: &mut Any, core: &mut Core) {
        if let Some(t) = object.downcast_mut() {
            (self.f)(t, core);
        }
    }
}

pub struct Core {
    io_handlers: Stash<Option<IoHandler>, Token>,
    objects: Stash<Option<Box<Any>>, ObjectId>,
    poll: Poll,
    exit: bool,
    process_handler: proc_imp::ProcessHandler,
}

impl Default for Core {
    fn default() -> Self {
        Self::new()
    }
}

impl Core {
    pub fn new() -> Core {
        proc_imp::new_core()
    }

    pub fn next_id(&self) -> ObjectId {
        self.objects.next_index()
    }

    pub fn add(&mut self, object: impl Any) -> ObjectId {
        self.objects.put(Some(Box::new(object)))
    }

    pub fn remove(&mut self, object_id: ObjectId) -> Option<Box<Any>> {
        self.objects.take(object_id).unwrap_or(None)
    }

    pub fn get<T: Any>(&self, object_id: ObjectId) -> Option<&T> {
        self.objects
            .get(object_id)
            .and_then(Option::as_ref)
            .map(Borrow::borrow)
            .and_then(Any::downcast_ref)
    }

    pub fn get_mut<T: Any>(&mut self, object_id: ObjectId) -> Option<&mut T> {
        self.objects
            .get_mut(object_id)
            .and_then(Option::as_mut)
            .map(BorrowMut::borrow_mut)
            .and_then(Any::downcast_mut)
    }

    pub fn register_reader<F, T>(&mut self, evented: &Evented, object_id: ObjectId, f: F)
    where
        F: 'static + FnMut(&mut T, &mut Core),
        T: Any,
    {
        self.internal_register(
            evented,
            Ready::readable(),
            object_id,
            Some(Box::new(Callback::new(f))),
            None,
        );
    }

    pub fn register_writer<F, T>(&mut self, evented: &Evented, object_id: ObjectId, f: F)
    where
        F: 'static + FnMut(&mut T, &mut Core),
        T: Any,
    {
        self.internal_register(
            evented,
            Ready::writable(),
            object_id,
            None,
            Some(Box::new(Callback::new(f))),
        );
    }

    pub fn register_reader_writer<FR, FW, T>(
        &mut self,
        evented: &Evented,
        object_id: ObjectId,
        f_read: FR,
        f_write: FW,
    ) where
        FR: 'static + FnMut(&mut T, &mut Core),
        FW: 'static + FnMut(&mut T, &mut Core),
        T: Any,
    {
        self.internal_register(
            evented,
            Ready::readable() | Ready::writable(),
            object_id,
            Some(Box::new(Callback::new(f_read))),
            Some(Box::new(Callback::new(f_write))),
        );
    }

    pub fn register_reaper<F, T, S>(&mut self, child: &Child<S>, object_id: ObjectId, f: F)
    where
        F: 'static + FnMut(&mut T, &mut Core),
        T: Any,
    {
        proc_imp::register_reaper(self, child, object_id, f);
    }

    pub fn run(&mut self) {
        let mut mio_events = MioEvents::with_capacity(32);
        loop {
            if self.exit || self.io_handlers.is_empty() {
                break;
            }
            trace!("About to sleep and wait for IO events.");
            self.poll.poll(&mut mio_events, None).unwrap();
            for event in &mio_events {
                let token = event.token();
                let mut io_handler = match self.io_handlers.get_mut(token).and_then(Option::take) {
                    Some(handler) => handler,
                    None => continue,
                };
                let obj_exists = self.call_on_object(io_handler.object_id, |object, core| {
                    if let Some(read_fn) = &mut io_handler.read_fn {
                        if event.readiness().is_readable() {
                            read_fn.make_call(object, core);
                        }
                    }
                    if let Some(write_fn) = &mut io_handler.write_fn {
                        if event.readiness().is_writable() {
                            write_fn.make_call(object, core);
                        }
                    }
                });
                if !obj_exists {
                    self.io_handlers.take(token);
                }
                if let Some(option) = self.io_handlers.get_mut(token) {
                    *option = Some(io_handler);
                }
            }
        }
    }

    pub fn exit(&mut self) {
        self.exit = true;
    }

    /// Starts running the given command.
    ///
    /// All three of stdin, stdout and stderr will be piped to/from this process.
    pub fn spawn(&self, mut cmd: impl BorrowMut<Command>) -> io::Result<Child<Stdin>> {
        // this is a method on core which takes a self parameter just to ensure that
        // a Core instance has been created first, needed for unix imp to register
        // a signal handler.
        let cmd = cmd.borrow_mut();
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        proc_imp::new_child(cmd.spawn()?)
    }

    fn call_on_object(&mut self, object_id: ObjectId, f: impl FnOnce(&mut Any, &mut Core)) -> bool {
        if let Some(mut box_object) = self.objects.get_mut(object_id).and_then(Option::take) {
            f(box_object.borrow_mut(), self);
            if let Some(option) = self.objects.get_mut(object_id) {
                *option = Some(box_object);
                return true;
            }
        }
        false
    }

    fn internal_register(
        &mut self,
        e: &Evented,
        r: Ready,
        object_id: ObjectId,
        read_fn: Option<Box<Call>>,
        write_fn: Option<Box<Call>>,
    ) {
        let token = self.io_handlers.next_index();
        self.poll.register(e, token, r, PollOpt::edge()).unwrap();
        self.io_handlers.put(Some(IoHandler {
            object_id,
            read_fn,
            write_fn,
        }));
    }
}

#[path = "process_unix.rs"]
#[cfg(unix)]
mod proc_imp;

#[path = "process_win.rs"]
#[cfg(windows)]
mod proc_imp;

pub use proc_imp::{Stderr, Stdin, Stdout};

pub struct Child<S> {
    child: ProcessChild,
    pub stdin: S,
    pub stdout: Stdout,
    pub stderr: Stderr,
}

impl<S> Child<S> {
    /// Returns the OS-assigned process identifier associated with this child.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Forces the child to exit.
    ///
    /// This is equivalent to sending a SIGKILL on unix platforms.
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }
}

impl Child<Stdin> {
    pub fn close_stdin(self) -> Child<()> {
        Child {
            child: self.child,
            stdin: (),
            stdout: self.stdout,
            stderr: self.stderr,
        }
    }
}
