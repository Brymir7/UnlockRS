use std::net::{ SocketAddr, UdpSocket };
use std::collections::HashMap;
use rand::seq;
use types::{ MsgBuffer, ServerPlayerID, NetworkMessage, ServerSideMessage, SeqNum };
mod type_impl;
mod types;
mod memory;

struct Server {
    socket: UdpSocket,
    addr_to_player: HashMap<SocketAddr, ServerPlayerID>,
    msg_buffer: MsgBuffer,
    last_processed_seq: HashMap<SocketAddr, SeqNum>,
    sequence_number: u8,
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
            last_processed_seq: HashMap::new(),
            sequence_number: 0,
        }
    }

    pub fn update(&mut self) {
        self.msg_buffer.clear();
        let (amt, src) = self.socket.recv_from(&mut self.msg_buffer.0).unwrap();

        if !self.addr_to_player.contains_key(&src) {
            self.create_new_connection(&src);
        }

        let msg = self.msg_buffer.parse_on_server();
        println!("Received msg {:?}", msg);

        if let Ok(server_side_msg) = msg {
            self.handle_message(server_side_msg, &src);
        }
    }

    pub fn create_new_connection(&mut self, addr: &SocketAddr) {
        let new_id = ServerPlayerID(self.addr_to_player.len() as u8);
        self.addr_to_player.insert(*addr, new_id);
        self.last_processed_seq.insert(*addr, SeqNum(0));
    }

    pub fn handle_message(&mut self, msg: ServerSideMessage, src: &SocketAddr) {
        if let Some(seq_num) = msg.seq_num {
            self.process_message(msg.msg, src);
            self.last_processed_seq.insert(*src, SeqNum(seq_num));
            self.send_ack(SeqNum(seq_num), src);
        } else {
            self.process_message(msg.msg, src);
        }
    }

    fn process_message(&mut self, msg: NetworkMessage, src: &SocketAddr) {
        match msg {
            NetworkMessage::SendWorldState(data) => {
                // Handle world state update
                println!("Processing world state update from {:?}", src);
                // Add your world state update logic here
            }
            NetworkMessage::SendPlayerInputs(inputs) => {
                // Handle player inputs
                println!("Processing player inputs from {:?}: {:?}", src, inputs);

                // Add your player input handling logic here
            }
            NetworkMessage::GetServerPlayerIDs => {
                println!("Request for player IDS");
                self.send_reliable(
                    NetworkMessage::SendServerPlayerIDs(
                        self.addr_to_player
                            .values()
                            .into_iter()
                            .map(|v| v.0)
                            .collect()
                    ),
                    src
                );
            }
            _ => {
                println!("Got some other message on server");
            }
            // Add other message types as needed
        }
    }

    fn send_ack(&mut self, seq_num: SeqNum, dst: &SocketAddr) {
        let serialized_msg = NetworkMessage::ServerSideAck(seq_num).serialize(
            types::NetworkMessageType::Unreliable
        );
        if let Err(e) = self.socket.send_to(&serialized_msg.bytes, dst) {
            eprintln!("Failed to send ACK to {:?}: {}", dst, e);
        }
    }
    pub fn send_reliable(&mut self, msg: NetworkMessage, dst: &SocketAddr) {
        let serialized_msg = msg.serialize(
            types::NetworkMessageType::Reliable(SeqNum(self.sequence_number))
        );
        self.sequence_number = self.sequence_number.wrapping_add(1);
        if let Err(e) = self.socket.send_to(&serialized_msg.bytes, dst) {
            eprintln!("Failed to send reliable message to {:?}: {}", dst, e);
        }
    }

    pub fn send_unreliable(&self, msg: NetworkMessage, dst: &SocketAddr) {
        let serialized_msg = msg.serialize(types::NetworkMessageType::Unreliable);
        if let Err(e) = self.socket.send_to(&serialized_msg.bytes, dst) {
            eprintln!("Failed to send unreliable message to {:?}: {}", dst, e);
        }
    }
}

fn main() -> std::io::Result<()> {
    let mut server = Server::new();
    loop {
        server.update();
    }
}
