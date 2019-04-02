#[macro_use]
extern crate log;
extern crate mio;
extern crate stash;

use mio::{Evented, Events as MioEvents, Poll, PollOpt, Ready, Token};
use stash::Stash;
use std::any::TypeId;
use std::borrow::BorrowMut;
use std::marker::PhantomData;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectId(u16);

impl From<usize> for ObjectId {
    fn from(idx: usize) -> Self {
        if idx > ::std::u16::MAX as usize {
            panic!("index type overflowing!");
        }
        ObjectId(idx as u16)
    }
}
impl Into<usize> for ObjectId {
    fn into(self) -> usize {
        self.0 as usize
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

enum IoHandler {
    Read(Box<Call>),
}

struct Callback<F, T> {
    f: F,
    object_id: ObjectId,
    _marker: PhantomData<T>,
}

trait Call {
    fn make_call(&self, &mut Core);
}

impl<F, T> Call for Callback<F, T>
where
    F: Fn(&mut T, &mut Core),
    T: Object,
    Box<Object>: BorrowMut<T>,
{
    fn make_call(&self, core: &mut Core) {
        if let Some(mut target) = core.take(self.object_id) {
            let a: &mut T = target.borrow_mut();
            if let Some(t) = Object::downcast_mut(a) {
                (self.f)(t, core);
            }
            //FIXME: re-insert the target
        }
    }
}

pub struct Core {
    io_handlers: Stash<IoHandler, Token>,
    objects: Stash<Option<Box<Object>>, ObjectId>,
    poll: Poll,
    exit: bool,
    interest: Vec<(Token, Token)>,
}

impl Core {
    pub fn new() -> Core {
        Core {
            io_handlers: Stash::default(),
            objects: Stash::default(),
            poll: Poll::new().unwrap(),
            exit: false,
            interest: Vec::new(),
        }
    }

    pub fn next_object_id(&self) -> ObjectId {
        self.objects.next_index()
    }

    pub fn add_object(&mut self, object: Box<Object>) -> ObjectId {
        self.objects.put(Some(object))
    }
    pub fn take(&mut self, object_id: ObjectId) -> Option<Box<Object>> {
        self.objects.get_mut(object_id).and_then(Option::take)
    }
    pub fn insert_object(&mut self, object_id: ObjectId, object: Box<Object>) {
        self.objects
            .get_mut(object_id)
            .and_then(|o| o.replace(object));
    }
    //add_interest<F, T>(token: Token, object_id: ObjectId, f: F)
    //    where F: FnMut(&mut T) {
    //        let handler = Handler{f, object_id, _marker: PhantomData{}};
    //
    //    }

    pub fn register_reader<F, T>(&mut self, e: &Evented, object_id: ObjectId, f: F) -> Token
    where
        F: 'static + Fn(&mut T, &mut Core),
        T: Object,
        Box<Object>: BorrowMut<T>,
    {
        let token = self.io_handlers.next_index();
        self.poll
            .register(e, token, Ready::readable(), PollOpt::edge())
            .unwrap();
        let callback = Callback {
            f,
            object_id,
            _marker: PhantomData,
        };
        self.io_handlers.put(IoHandler::Read(Box::new(callback)))
    }

    pub fn register_interest(&mut self, caller: Token, subject: Token) {
        self.interest.push((caller, subject));
    }

    //    pub fn remove(&mut self, token: Token) {
    //        if let Some(Some(handler)) = self.io_handlers.take(token) {
    //            self.poll.deregister(&*handler).unwrap();
    //        }
    //        let mut i = 0;
    //        while i < self.interest.len() {
    //            let (caller, subject) = self.interest[i];
    //            if subject == token {
    //                if let Some(Some(handler)) = self.io_handlers.get_mut(caller) {
    //                    handler.remove_token(subject);
    //                }
    //                self.interest.swap_remove(i);
    //            } else {
    //                i += 1;
    //            }
    //        }
    //    }

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
            //            for event in &mio_events {
            //                let mut handler = match self.io_handlers.get_mut(event.token()) {
            //                    Some(ref mut option) => match option.take() {
            //                        Some(handler) => handler,
            //                        None => continue,
            //                    },
            //                    // Possibly mio can return many events for the same token
            //                    // and the handler been removed by a previous event.
            //                    None => continue,
            //                };
            //
            //                if let Some(option) = self.io_handlers.get_mut(event.token()) {
            //                    *option = Some(handler);
            //                } else {
            //                    // if get_mut returns None then the handler must have removed itself
            //                    // so we just drop the handler itself at the end of scope (for loop).
            //                    self.poll.deregister(&*handler).unwrap();
            //                }
            //            }
        }
    }
}
