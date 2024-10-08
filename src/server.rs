use std::hash::Hash;
use std::net::{ SocketAddr, UdpSocket };
use std::collections::HashMap;
use types::{ MsgBuffer, ServerPlayerID, NetworkMessage, MAX_UDP_PAYLOAD_LEN };
mod type_impl;
mod types;
mod memory;
struct Server {
    socket: UdpSocket,
    addr_to_player: HashMap<SocketAddr, ServerPlayerID>,
    msg_buffer: MsgBuffer,
}
impl Server {
    pub fn new() -> Self {
        let addr_to_player: HashMap<SocketAddr, ServerPlayerID> = HashMap::new();
        let socket = UdpSocket::bind("127.0.0.1:8080").expect("Server Failed to bind socket.");
        let msg_buffer: MsgBuffer = MsgBuffer::default();
        Server {
            socket,
            addr_to_player,
            msg_buffer,
        }
    }
    pub fn update(&mut self) {
        self.msg_buffer.clear();
        let (amt, src) = self.socket.recv_from(&mut self.msg_buffer.0).unwrap();
        if !self.addr_to_player.contains_key(&src) {
            self.create_new_connection(&src);
        }
        let msg = self.msg_buffer.parse_message();
        println!("Received msg {:?}", msg);
        if let Ok(req) = msg {
            self.handle_request(req);
        }
        self.socket.send_to(&mut self.msg_buffer.0[..amt], src).unwrap();
    }

    pub fn create_new_connection(&mut self, addr: &SocketAddr) {}

    pub fn handle_request(&self, req: NetworkMessage) {}
}

fn main() -> std::io::Result<()> {
    let mut server = Server::new();
    loop {
        server.update();
    }
}
