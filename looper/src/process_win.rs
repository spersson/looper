use crate::{Call, Callback, Child, Core, ObjectId};
use log::error;
use mio::Poll;
use mio_extras::channel::{channel, Receiver, Sender};
use mio_named_pipes::NamedPipe;
use stash::Stash;
use std::any::Any;
use std::collections::VecDeque;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle};
use std::process;
use winapi::um::handleapi::INVALID_HANDLE_VALUE;
use winapi::um::synchapi::WaitForSingleObject;
use winapi::um::threadpoollegacyapiset::UnregisterWaitEx;
use winapi::um::winbase::{RegisterWaitForSingleObject, INFINITE, WAIT_OBJECT_0};
use winapi::um::winnt::{BOOLEAN, HANDLE, PVOID, WT_EXECUTEINWAITTHREAD, WT_EXECUTEONLYONCE};

pub fn new_core() -> Core {
    let (sender, receiver) = channel();
    let mut core = Core {
        io_handlers: Stash::default(),
        objects: Stash::default(),
        poll: Poll::new().unwrap(),
        exit: false,
        process_handler: ProcessHandler {
            reapers: VecDeque::new(),
            sender,
        },
    };
    core.register_reader(&receiver, core.next_id(), reap);
    core.add(receiver);
    core
}

struct Sentinel {
    id: u32,
    sender: Sender<u32>,
}

impl Sentinel {
    fn send(&mut self) {
        self.sender.send(self.id).unwrap();
    }
}

unsafe extern "system" fn callback(ptr: PVOID, _timer_fired: BOOLEAN) {
    eprintln!("callback called");
    let sentinel = &mut *(ptr as *mut Sentinel);
    sentinel.send();
}

struct Reaper {
    wait_object: Option<HANDLE>,
    sentinel: Box<Sentinel>,
    object_id: ObjectId,
    callback: Box<Call>,
}

impl Drop for Reaper {
    fn drop(&mut self) {
        if let Some(handle) = self.wait_object {
            let rc = unsafe { UnregisterWaitEx(handle, INVALID_HANDLE_VALUE) };
            if rc == 0 {
                error!("failed to unregister: {}", io::Error::last_os_error());
            }
        }
    }
}
pub struct ProcessHandler {
    reapers: VecDeque<Reaper>,
    sender: Sender<u32>,
}

fn reap(receiver: &mut Receiver<u32>, core: &mut Core) {
    while let Ok(id) = receiver.try_recv() {
        for _ in 0..core.process_handler.reapers.len() {
            let r = core.process_handler.reapers.pop_front().unwrap();
            if r.sentinel.id == id {
                core.call_on_object(r.object_id, |obj, c| r.callback.make_call(obj, c));
            } else {
                core.process_handler.reapers.push_back(r);
            }
        }
    }
}

pub fn register_reaper<F, T>(core: &mut Core, child: &Child, object_id: ObjectId, f: F)
where
    F: 'static + Fn(&mut T, &mut Core),
    T: Any,
{
    let res = unsafe { WaitForSingleObject(child.child.as_raw_handle(), 0) };
    let mut sentinel = Box::new(Sentinel {
        sender: core.process_handler.sender.clone(),
        id: child.child.id(),
    });
    let reaper = if res == WAIT_OBJECT_0 {
        sentinel.send();
        Reaper {
            sentinel,
            wait_object: None,
            object_id,
            callback: Box::new(Callback::new(f)),
        }
    } else {
        let ptr = Box::into_raw(sentinel);
        let mut wait_object = 0 as *mut _;
        let rc = unsafe {
            RegisterWaitForSingleObject(
                &mut wait_object,
                child.child.as_raw_handle(),
                Some(callback),
                ptr as *mut _,
                INFINITE,
                WT_EXECUTEINWAITTHREAD | WT_EXECUTEONLYONCE,
            )
        };
        let sentinel = unsafe { Box::from_raw(ptr) };
        if rc == 0 {
            error!(
                "Failed to register callback for process exit: {}",
                io::Error::last_os_error()
            );
            return;
        }

        Reaper {
            sentinel,
            wait_object: Some(wait_object),
            object_id,
            callback: Box::new(Callback::new(f)),
        }
    };
    core.process_handler.reapers.push_back(reaper);
}

pub type Stdin = NamedPipe;
pub type Stdout = NamedPipe;
pub type Stderr = NamedPipe;

pub fn new_child(mut child: process::Child) -> io::Result<Child> {
    let stdin = stdio(child.stdin.take().unwrap());
    let stdout = stdio(child.stdout.take().unwrap());
    let stderr = stdio(child.stderr.take().unwrap());
    Ok(Child {
        child,
        stdin,
        stdout,
        stderr,
    })
}

fn stdio<T: IntoRawHandle>(io: T) -> NamedPipe {
    unsafe { NamedPipe::from_raw_handle(io.into_raw_handle()) }
}
