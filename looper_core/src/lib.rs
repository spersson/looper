#[macro_use]
extern crate log;
extern crate mio;
extern crate slab;

use std::any::Any;

use mio::event::Evented as MioEvented;
use mio::{Events as MioEvents, Poll, PollOpt, Ready, Token};
use slab::Slab;

pub trait IoHandler<S>: MioEvented + Any {
    fn store_token(&mut self, token: Token);
    fn read_all(&mut self, _core: &mut Core<S>, _state: &mut S) {}
    fn write_all(&mut self, _core: &mut Core<S>, _state: &mut S) {}
}

pub struct Core<S> {
    handlers: Slab<Option<Box<IoHandler<S>>>>,
    poll: Poll,
    exit: bool,
}

impl<S> Core<S> {
    pub fn new() -> Core<S> {
        Core {
            handlers: Slab::with_capacity(8),
            poll: Poll::new().unwrap(),
            exit: false,
        }
    }

    pub fn add_io_handler(&mut self, mut handler: Box<IoHandler<S>>) -> Token {
        let next = self.handlers.vacant_entry();
        self.poll
            .register(
                &*handler,
                Token(next.key()),
                Ready::readable() | Ready::writable(),
                PollOpt::edge(),
            )
            .unwrap();
        handler.store_token(Token(next.key()));
        let result = next.key();
        next.insert(Some(handler));
        result.into()
    }

    pub fn get_io_handler(&mut self, token: Token) -> Option<&mut Box<IoHandler<S>>> {
        self.handlers.get_mut(token.into()).and_then(Option::as_mut)
    }

    pub fn remove_io_handler(&mut self, token: Token) {
        if self.handlers.contains(token.into()) {
            let option = self.handlers.remove(token.into());
            if let Some(handler) = option {
                self.poll.deregister(&*handler).unwrap();
            }
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
                let token = usize::from(event.token());
                let mut handler = match self.handlers.get_mut(token) {
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
                if let Some(option) = self.handlers.get_mut(token) {
                    *option = Some(handler);
                }
            }
        }
        state
    }
}
