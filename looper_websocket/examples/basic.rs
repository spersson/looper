extern crate looper_core;
extern crate looper_websocket;

use looper_core::Core;
use looper_websocket::{WebSocketHandler, WebSocketServer};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

struct Client;

impl WebSocketHandler for Client {
    fn welcome_message(&mut self, _core: &mut Core) -> Option<String> {
        Some(String::from("Hello there!"))
    }

    fn handle_message(&mut self, message: String, _core: &mut Core) -> Option<String> {
        Some(format!("I heard you say: {}", message))
    }
}

fn main() {
    let mut core = Core::new();
    let web_socket_address =
        SocketAddr::from(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 17771));

    let _server_id = WebSocketServer::new(web_socket_address, || Client, &mut core)
        .expect("Port 17771 expected to be available.");

    core.run();
}
