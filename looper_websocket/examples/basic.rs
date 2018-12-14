extern crate looper_core;
extern crate looper_websocket;

use looper_core::Core;
use looper_websocket::{WebSocketHandler, WebSocketServer};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

struct World;
struct Client;

impl WebSocketHandler<World> for Client {}

fn main() {
    let mut core = Core::new();
    let web_socket_address =
        SocketAddr::from(SocketAddrV4::new(Ipv4Addr::new(127, 0, 0, 1), 17771));

    let server = WebSocketServer::new(web_socket_address, || Client, core.next_token()).unwrap();
    core.insert(Box::new(server));

    let state = World;
    core.run(state);
}
