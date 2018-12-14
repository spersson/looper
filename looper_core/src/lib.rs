#[macro_use]
extern crate log;
extern crate mio;
extern crate stash;

use mio::{Evented, Events, Poll, PollOpt, Ready, Token};
use stash::Stash;
use std::any::TypeId;
use std::borrow::BorrowMut;

pub trait IoHandler<S>: 'static {
    fn event_source(&self) -> &Evented;
    fn read_all(&mut self, _core: &mut Core<S>, _state: &mut S) {}
    fn write_all(&mut self, _core: &mut Core<S>, _state: &mut S) {}
    fn remove_token(&mut self, _token: Token) {}
}

impl<S: 'static> IoHandler<S> {
    fn downcast_mut<T: IoHandler<S>>(&mut self) -> Option<&mut T> {
        if TypeId::of::<T>() == TypeId::of::<Self>() {
            Some(unsafe { &mut *(self as *mut Self as *mut T) })
        } else {
            None
        }
    }
}

pub struct Core<S> {
    io_handlers: Stash<Option<Box<IoHandler<S>>>, Token>,
    poll: Poll,
    exit: bool,
    interest: Vec<(Token, Token)>,
}

impl<S: 'static> Core<S> {
    pub fn new() -> Core<S> {
        Core {
            io_handlers: Stash::default(),
            poll: Poll::new().unwrap(),
            exit: false,
            interest: Vec::new(),
        }
    }

    pub fn next_token(&self) -> Token {
        self.io_handlers.next_index()
    }

    pub fn insert(&mut self, io_handler: Box<IoHandler<S>>) -> Token {
        self.poll
            .register(
                io_handler.event_source(),
                self.next_token(),
                Ready::readable() | Ready::writable(),
                PollOpt::edge(),
            )
            .unwrap();
        self.io_handlers.put(Some(io_handler))
    }

    pub fn register_interest(&mut self, caller: Token, subject: Token) {
        self.interest.push((caller, subject));
    }

    pub fn get_mut<T: IoHandler<S>>(&mut self, token: Token) -> Option<&mut T> {
        self.io_handlers
            .get_mut(token)
            .and_then(Option::as_mut)
            .map(BorrowMut::borrow_mut)
            .and_then(IoHandler::downcast_mut)
    }

    pub fn remove(&mut self, token: Token) {
        if let Some(Some(handler)) = self.io_handlers.take(token) {
            self.poll.deregister(handler.event_source()).unwrap();
        }
        let mut i = 0;
        while i < self.interest.len() {
            let (caller, subject) = self.interest[i];
            if subject == token {
                if let Some(Some(handler)) = self.io_handlers.get_mut(caller) {
                    handler.remove_token(subject);
                }
                self.interest.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    pub fn exit(&mut self) {
        self.exit = true;
    }

    pub fn run(&mut self, mut state: S) -> S {
        let mut mio_events = Events::with_capacity(32);
        loop {
            if self.exit {
                break;
            }
            trace!("About to sleep and wait for IO events.");
            self.poll.poll(&mut mio_events, None).unwrap();
            for event in &mio_events {
                let mut handler = match self.io_handlers.get_mut(event.token()) {
                    Some(ref mut option) => match option.take() {
                        Some(handler) => handler,
                        None => continue,
                    },
                    // Possibly mio can return many events for the same token
                    // and the handler been removed by a previous event.
                    None => continue,
                };

                if event.readiness().is_readable() {
                    handler.read_all(self, &mut state);
                }
                if event.readiness().is_writable() {
                    handler.write_all(self, &mut state);
                }

                if let Some(option) = self.io_handlers.get_mut(event.token()) {
                    *option = Some(handler);
                } else {
                    // if get_mut returns None then the handler must have removed itself
                    // so we just drop the handler itself at the end of scope (for loop).
                    self.poll.deregister(handler.event_source()).unwrap();
                }
            }
        }
        state
    }
}
