#[macro_use]
extern crate log;
extern crate mio;
extern crate slab;

use std::collections::VecDeque;
use std::any::Any;

use mio::event::Evented as MioEvented;
use mio::{Events as MioEvents, Poll, PollOpt, Ready, Token};
use slab::Slab;

pub trait IoHandler<S>: MioEvented {
    fn store_token(&mut self, token: Token);
    fn read_all(&mut self, _queue: &mut EventQueue<S>, _state: &mut S) {}
    fn write_all(&mut self, _queue: &mut EventQueue<S>, _state: &mut S) {}
    fn custom_event(&mut self, _details: Box<Any>, _queue: &mut EventQueue<S>, _state: &mut S) {}
}

enum Event<S> {
    AddIoHandler(Box<IoHandler<S>>),
    RemoveIoHandler(Token),
    Custom(Token, Box<Any>),
    Exit,
}

pub struct EventQueue<S> {
    inner: VecDeque<Event<S>>,
}

impl<S> EventQueue<S> {
    pub fn new() -> EventQueue<S> {
        EventQueue {
            inner: VecDeque::new(),
        }
    }

    pub fn add_io_handler(&mut self, handler: Box<IoHandler<S>>) {
        self.inner.push_back(Event::AddIoHandler(handler));
    }

    pub fn remove_io_handler(&mut self, token: Token) {
        self.inner.push_back(Event::RemoveIoHandler(token));
    }

    pub fn add_custom_event(&mut self, token: Token, details: Box<Any>) {
        self.inner.push_back(Event::Custom(token, details));
    }

    pub fn exit(&mut self) {
        self.inner.push_back(Event::Exit);
    }
}

pub fn event_loop<S>(mut queue: EventQueue<S>, mut state: S) -> S {
    let poll = Poll::new().unwrap();
    let mut mio_events = MioEvents::with_capacity(32);
    let mut io_handlers = Slab::with_capacity(4);
    loop {
        while let Some(event) = queue.inner.pop_front() {
            match event {
                Event::Exit => {
                    debug!("Event loop exiting after receiving an Exit event.");
                    return state;
                }
                Event::AddIoHandler(mut handler) => {
                    let next = io_handlers.vacant_entry();
                    poll.register(
                        &*handler,
                        Token(next.key()),
                        Ready::readable() | Ready::writable(),
                        PollOpt::edge(),
                    ).unwrap();
                    handler.store_token(Token(next.key()));
                    next.insert(handler);
                }
                Event::RemoveIoHandler(token) => {
                    if io_handlers.contains(token.into()) {
                        let handler = io_handlers.remove(token.into());
                        poll.deregister(&*handler).unwrap();
                    }
                }
                Event::Custom(token, details) => {
                    if let Some(handler) = io_handlers.get_mut(token.into()) {
                        handler.custom_event(details, &mut queue, &mut state);
                    }
                }
            }
        }
        trace!("About to sleep and wait for IO events.");
        poll.poll(&mut mio_events, None).unwrap();
        for event in &mio_events {
            if let Some(handler) = io_handlers.get_mut(usize::from(event.token())) {
                if event.readiness().is_readable() {
                    handler.read_all(&mut queue, &mut state);
                }
                if event.readiness().is_writable() {
                    handler.write_all(&mut queue, &mut state);
                }
            }
        }
    }
}
