#[macro_use]
extern crate log;
extern crate mio;
extern crate stash;

use std::any::Any;

use mio::event::Evented as MioEvented;
use mio::{Events as MioEvents, Poll, PollOpt, Ready, Token};
use stash::Stash;

pub trait IoHandler<S>: MioEvented + Any {
    fn read_all(&mut self, _core: &mut Core<S>, _state: &mut S) {}
    fn write_all(&mut self, _core: &mut Core<S>, _state: &mut S) {}
}

pub struct Core<S> {
    io_handlers: Stash<Option<Box<IoHandler<S>>>, Token>,
    poll: Poll,
    exit: bool,
}

impl<S> Core<S> {
    pub fn new() -> Core<S> {
        Core {
            io_handlers: Stash::default(),
            poll: Poll::new().unwrap(),
            exit: false,
        }
    }

    pub fn next_token(&self) -> Token {
        self.io_handlers.next_index()
    }

    pub fn insert(&mut self, io_handler: Box<IoHandler<S>>) -> Token {
        self.poll
            .register(
                &*io_handler,
                self.next_token(),
                Ready::readable() | Ready::writable(),
                PollOpt::edge(),
            )
            .unwrap();
        self.io_handlers.put(Some(io_handler))
    }

    pub fn get_mut(&mut self, token: Token) -> Option<&mut Box<IoHandler<S>>> {
        self.io_handlers.get_mut(token).and_then(Option::as_mut)
    }

    pub fn remove(&mut self, token: Token) {
        if let Some(Some(handler)) = self.io_handlers.take(token) {
            self.poll.deregister(&*handler).unwrap();
        }
    }

    pub fn exit(&mut self) {
        self.exit = true;
    }

    pub fn run(&mut self, mut state: S) -> S {
        let mut mio_events = MioEvents::with_capacity(32);
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
                    None => continue,
                };

                if event.readiness().is_readable() {
                    handler.read_all(self, &mut state);
                }
                if event.readiness().is_writable() {
                    handler.write_all(self, &mut state);
                }

                // if get_mut returns None then the handler must have removed itself from self.handlers
                // so we just drop the handler itself at the end of scope (loop).
                if let Some(option) = self.io_handlers.get_mut(event.token()) {
                    *option = Some(handler);
                }
            }
        }
        state
    }
}
