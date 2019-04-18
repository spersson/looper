extern crate looper_core;
#[macro_use]
extern crate log;
extern crate mio;
extern crate tungstenite;

use looper_core::{Core, ObjectId};
use mio::net::{TcpListener, TcpStream};
use std::io::{ErrorKind, Result};
use std::net::SocketAddr;
use tungstenite::{server, Error as InnerSocketError, Message, WebSocket as InnerSocket};

pub trait WebSocketHandler {
    fn acceptable(&mut self, _from_address: SocketAddr) -> bool {
        true
    }

    fn welcome_message(&mut self, _core: &mut Core) -> Option<String> {
        None
    }

    fn handle_message(&mut self, _message: String, _core: &mut Core) -> Option<String> {
        None
    }
}

pub struct WebSocketServer<F> {
    tcp_listener: TcpListener,
    factory: F,
    object_id: ObjectId,
    sockets: Vec<ObjectId>,
}

impl<W, F> WebSocketServer<F>
where
    W: 'static + WebSocketHandler,
    F: 'static + Fn() -> W,
{
    pub fn start(socket_address: SocketAddr, factory: F, core: &mut Core) -> Result<ObjectId> {
        let tcp_listener = TcpListener::bind(&socket_address)?;
        let object_id = core.next_id();
        core.register_reader(&tcp_listener, object_id, WebSocketServer::<F>::read_all);
        let server = WebSocketServer {
            tcp_listener,
            factory,
            object_id,
            sockets: Vec::new(),
        };
        core.add(Box::new(server));
        Ok(object_id)
    }

    pub fn broadcast(&self, core: &mut Core, message: String) {
        for id in &self.sockets {
            if let Some(socket) = core.get_mut::<WebSocket<W>>(*id) {
                socket
                    .inner_socket
                    .write_message(Message::Text(message.clone()))
                    .unwrap();
            }
        }
    }

    fn read_all(&mut self, core: &mut Core) {
        loop {
            let (tcp_stream, address) = match self.tcp_listener.accept() {
                Ok((t, a)) => (t, a),
                Err(ref e) => {
                    if e.kind() != ErrorKind::WouldBlock {
                        error!("Error while trying to accept an incoming connection: {}", e);
                        core.remove(self.object_id);
                    }
                    return;
                }
            };
            let mut handler = (self.factory)();
            if !handler.acceptable(address) {
                info!(
                    "Connection from {} found unacceptable. Dropping it.",
                    address
                );
                continue; // just drop the tcp stream
            }
            let mut inner_socket = match server::accept(tcp_stream) {
                Ok(inner_socket) => inner_socket,
                Err(err) => {
                    error!("Failed to open a new websocket: {}", err);
                    continue;
                }
            };
            if let Some(message) = handler.welcome_message(core) {
                inner_socket.write_message(Message::Text(message)).unwrap();
            }
            let object_id = core.next_id();
            core.register_reader_writer(
                inner_socket.get_ref(),
                object_id,
                WebSocket::<W>::read_all,
                WebSocket::<W>::write_all,
            );
            core.add(Box::new(WebSocket {
                inner_socket,
                handler,
                object_id,
            }));
            self.sockets.push(object_id);
        }
    }
}

struct WebSocket<W> {
    inner_socket: InnerSocket<TcpStream>,
    handler: W,
    object_id: ObjectId,
}

impl<W> WebSocket<W>
where
    W: 'static + WebSocketHandler,
{
    fn read_all(&mut self, core: &mut Core) {
        loop {
            match self.inner_socket.read_message() {
                Err(InnerSocketError::ConnectionClosed(_)) => {
                    info!("Connection closed.");
                    core.remove(self.object_id);
                    return;
                }
                Err(InnerSocketError::Io(err)) => {
                    if err.kind() != ErrorKind::WouldBlock {
                        error!("IO error while trying to read incoming message: {}", err);
                        core.remove(self.object_id);
                    }
                    return;
                }
                Err(err) => {
                    error!(
                        "Non-fatal error while trying to read an incoming message: {}",
                        err
                    );
                }
                Ok(Message::Text(message)) => {
                    if let Some(reply) = self.handler.handle_message(message, core) {
                        self.inner_socket
                            .write_message(Message::Text(reply))
                            .unwrap();
                    }
                }
                Ok(_other) => warn!("Received and ignored message because it was not text-type."),
            }
        }
    }

    fn write_all(&mut self, core: &mut Core) {
        match self.inner_socket.write_pending() {
            Err(InnerSocketError::Io(err)) => {
                if err.kind() != ErrorKind::WouldBlock {
                    error!("Error while trying to write outgoing message: {}", err);
                    core.remove(self.object_id);
                }
            }
            Err(InnerSocketError::ConnectionClosed(_)) => {
                info!("Connection closed.");
                core.remove(self.object_id);
            }
            Err(err) => error!("Error while trying to write an outgoing message: {}", err),
            Ok(()) => debug!("Successfully flushed pending messages to send."),
        }
    }
}
