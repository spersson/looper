use log::trace;
use mio::{Evented, Events as MioEvents, Poll, PollOpt, Ready, Token};
use stash::Stash;
use std::any::TypeId;
use std::borrow::BorrowMut;
use std::marker::PhantomData;

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

pub trait Object: 'static {}

impl<T: 'static> Object for T {}

impl Object {
    fn downcast_mut<T: Object>(&mut self) -> Option<&mut T> {
        if TypeId::of::<T>() == TypeId::of::<Self>() {
            Some(unsafe { &mut *(self as *mut Self as *mut T) })
        } else {
            None
        }
    }
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

trait Call {
    fn make_call(&self, _: &mut Object, _: &mut Core);
}

impl<F, T> Call for Callback<F, T>
where
    F: Fn(&mut T, &mut Core),
    T: Object,
{
    fn make_call(&self, object: &mut Object, core: &mut Core) {
        if let Some(t) = Object::downcast_mut(object) {
            (self.f)(t, core);
        }
    }
}

pub struct Core {
    io_handlers: Stash<Option<IoHandler>, Token>,
    objects: Stash<Option<Box<Object>>, ObjectId>,
    poll: Poll,
    exit: bool,
}

impl Core {
    pub fn new() -> Core {
        Core {
            io_handlers: Stash::default(),
            objects: Stash::default(),
            poll: Poll::new().unwrap(),
            exit: false,
        }
    }

    pub fn next_object_id(&self) -> ObjectId {
        self.objects.next_index()
    }

    pub fn add_object(&mut self, object: Box<Object>) -> ObjectId {
        self.objects.put(Some(object))
    }

    pub fn remove_object(&mut self, object_id: ObjectId) -> Option<Box<Object>> {
        self.objects.take(object_id).unwrap_or(None)
    }

    pub fn get_mut<T: Object>(&mut self, object_id: ObjectId) -> Option<&mut T> {
        self.objects
            .get_mut(object_id)
            .and_then(Option::as_mut)
            .map(BorrowMut::borrow_mut)
            .and_then(Object::downcast_mut)
    }

    pub fn register_reader<F, T>(&mut self, e: &Evented, object_id: ObjectId, f: F)
    where
        F: 'static + Fn(&mut T, &mut Core),
        T: Object,
    {
        self.internal_register(
            e,
            Ready::readable(),
            object_id,
            Some(Box::new(Callback::new(f))),
            None,
        );
    }

    pub fn register_writer<F, T>(&mut self, e: &Evented, object_id: ObjectId, f: F)
    where
        F: 'static + Fn(&mut T, &mut Core),
        T: Object,
    {
        self.internal_register(
            e,
            Ready::writable(),
            object_id,
            None,
            Some(Box::new(Callback::new(f))),
        );
    }

    pub fn register_reader_writer<FR, FW, T>(
        &mut self,
        e: &Evented,
        object_id: ObjectId,
        f_read: FR,
        f_write: FW,
    ) where
        FR: 'static + Fn(&mut T, &mut Core),
        FW: 'static + Fn(&mut T, &mut Core),
        T: Object,
    {
        self.internal_register(
            e,
            Ready::readable() | Ready::writable(),
            object_id,
            Some(Box::new(Callback::new(f_read))),
            Some(Box::new(Callback::new(f_write))),
        );
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

    pub fn exit(&mut self) {
        self.exit = true;
    }

    pub fn run(&mut self) {
        let mut mio_events = MioEvents::with_capacity(32);
        loop {
            if self.exit {
                break;
            }
            trace!("About to sleep and wait for IO events.");
            self.poll.poll(&mut mio_events, None).unwrap();
            for event in &mio_events {
                let token = event.token();
                let io_handler = match self.io_handlers.get_mut(token).and_then(Option::take) {
                    Some(handler) => handler,
                    None => continue,
                };

                let mut box_object = match self
                    .objects
                    .get_mut(io_handler.object_id)
                    .and_then(Option::take)
                {
                    Some(object) => object,
                    None => {
                        // Possibly mio can return many events for the same token
                        // and the object been removed by a previous event. Remove the io handler also.
                        self.io_handlers.take(token);
                        continue;
                    }
                };
                if let Some(read_fn) = &io_handler.read_fn {
                    if event.readiness().is_readable() {
                        read_fn.make_call(box_object.borrow_mut(), self);
                    }
                }
                if let Some(write_fn) = &io_handler.write_fn {
                    if event.readiness().is_writable() {
                        write_fn.make_call(box_object.borrow_mut(), self);
                    }
                }

                if let Some(option) = self.objects.get_mut(io_handler.object_id) {
                    *option = Some(box_object);
                } else {
                    // if get_mut returns None then the object must have removed itself from Core.
                    // Just drop the object itself at the end of scope (for loop).
                    // Take the chance to remove the io_handler straight away.
                    self.io_handlers.take(token);
                }

                if let Some(option) = self.io_handlers.get_mut(token) {
                    *option = Some(io_handler);
                }
            }
        }
    }
}
