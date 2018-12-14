extern crate looper_core;
#[macro_use]
extern crate log;
extern crate mio;
extern crate tungstenite;

use std::io::{ErrorKind, Result};
use std::marker::PhantomData;
use std::net::SocketAddr;

use mio::{net::TcpListener, net::TcpStream, Evented, Token};
use tungstenite::{server, Error as InnerSocketError, Message, WebSocket as InnerSocket};

use looper_core::{Core, IoHandler};

pub trait WebSocketHandler<S> {
    fn acceptable(&mut self, _from_address: SocketAddr) -> bool {
        true
    }

    fn welcome_message(&mut self, _state: &mut S) -> Option<Message> {
        None
    }

    fn handle_message(&mut self, _message: Message, _state: &mut S) -> Option<Message> {
        None
    }
}

pub struct WebSocketServer<S, F> {
    tcp_listener: TcpListener,
    factory: F,
    token: Token,
    sockets: Vec<Token>,
    _marker: PhantomData<S>,
}

impl<S, W, F> WebSocketServer<S, F>
where
    S: 'static,
    W: 'static + WebSocketHandler<S>,
    F: 'static + Fn() -> W,
{
    pub fn new(
        socket_address: SocketAddr,
        factory: F,
        token: Token,
    ) -> Result<WebSocketServer<S, F>> {
        let tcp_listener = TcpListener::bind(&socket_address)?;
        Ok(WebSocketServer {
            tcp_listener,
            factory,
            token,
            sockets: Vec::new(),
            _marker: PhantomData,
        })
    }

    pub fn broadcast(&self, core: &mut Core<S>, message: Message) {
        for token in &self.sockets {
            if let Some(socket) = core.get_mut::<WebSocket<W>>(*token) {
                socket.inner_socket.write_message(message.clone()).unwrap();
            }
        }
    }
}

impl<S, W, F> IoHandler<S> for WebSocketServer<S, F>
where
    S: 'static,
    W: 'static + WebSocketHandler<S>,
    F: 'static + Fn() -> W,
{
    fn event_source(&self) -> &Evented {
        &self.tcp_listener
    }

    fn read_all(&mut self, core: &mut Core<S>, state: &mut S) {
        loop {
            let (tcp_stream, address) = match self.tcp_listener.accept() {
                Ok((t, a)) => (t, a),
                Err(ref e) => {
                    if e.kind() != ErrorKind::WouldBlock {
                        error!("Error while trying to accept an incoming connection: {}", e);
                        core.remove(self.token);
                    }
                    return;
                }
            };
            let mut handler = (self.factory)();
            if !handler.acceptable(address) {
                continue; // just drop the tcp stream
            }
            //FIXME: should handle errors
            if let Ok(mut inner_socket) = server::accept(tcp_stream) {
                if let Some(message) = handler.welcome_message(state) {
                    inner_socket.write_message(message).unwrap();
                }
                let token = core.next_token();
                let io_handler = Box::new(WebSocket {
                    inner_socket,
                    handler,
                    token,
                });
                core.insert(io_handler);
                core.register_interest(self.token, token);
                self.sockets.push(token);
            }
        }
    }

    fn remove_token(&mut self, token: Token) {
        self.sockets.retain(|t| *t != token);
    }
}

pub struct WebSocket<W> {
    inner_socket: InnerSocket<TcpStream>,
    handler: W,
    token: Token,
}

impl<S, W> IoHandler<S> for WebSocket<W>
where
    S: 'static,
    W: 'static + WebSocketHandler<S>,
{
    fn event_source(&self) -> &Evented {
        self.inner_socket.get_ref()
    }

    fn read_all(&mut self, core: &mut Core<S>, state: &mut S) {
        loop {
            match self.inner_socket.read_message() {
                Err(InnerSocketError::Io(ref e)) if e.kind() == ErrorKind::WouldBlock => return,
                Err(InnerSocketError::Utf8) => continue,
                Err(e) => {
                    match e {
                        InnerSocketError::ConnectionClosed(_) => (),
                        _ => {
                            error!("Error while trying to read an incoming message: {}", e);
                        }
                    }
                    core.remove(self.token);
                    return;
                }
                Ok(message) => {
                    if let Some(reply) = self.handler.handle_message(message, state) {
                        self.inner_socket.write_message(reply).unwrap();
                    }
                }
            }
        }
    }

    fn write_all(&mut self, core: &mut Core<S>, _state: &mut S) {
        if let Err(e) = self.inner_socket.write_pending() {
            match e {
                InnerSocketError::Io(ref e) if e.kind() == ErrorKind::WouldBlock => return,
                InnerSocketError::ConnectionClosed(_) => (),
                _ => {
                    error!("Error while trying to read an incoming message: {}", e);
                }
            }
            core.remove(self.token);
        }
    }
}
