use std::net::{ SocketAddr, UdpSocket };
use std::collections::HashMap;
use std::time::{ Duration, Instant };
use rand::seq;
use types::{
    MsgBuffer,
    NetworkMessage,
    SeqNum,
    SerializedNetworkMessage,
    ServerPlayerID,
    ServerSideMessage,
};
mod type_impl;
mod types;
mod memory;
const MAX_RETRIES: u32 = 20;
const RETRY_TIMEOUT: Duration = Duration::from_millis(100);
struct Server {
    socket: UdpSocket,
    addr_to_player: HashMap<SocketAddr, ServerPlayerID>,
    msg_buffer: MsgBuffer,
    pending_acks: HashMap<SocketAddr, HashMap<SeqNum, (Instant, SerializedNetworkMessage)>>,
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
            pending_acks: HashMap::new(),
            sequence_number: 0,
        }
    }

    pub fn update(&mut self) {
        self.msg_buffer.clear();
        match self.socket.recv_from(&mut self.msg_buffer.0) {
            Ok((_, src)) => {
                if !self.addr_to_player.contains_key(&src) {
                    self.create_new_connection(&src);
                    println!("addr to player doesnt ocntain key creating connection");
                    println!(
                        "{:?}",
                        self.addr_to_player
                            .values()
                            .map(|v| v.0) // Assuming `v.0` is a u8
                            .collect::<Vec<u8>>()
                    );
                }

                let msg = self.msg_buffer.parse_on_server();
                println!("Received msg {:?}", msg);
                if let Ok(server_side_msg) = msg {
                    self.handle_message(server_side_msg, &src);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                println!("No data available");
            }
            Err(e) => {
                eprintln!("Error receiving data: {}", e);
            }
        }
        self.handle_retransmissions();
    }
    pub fn handle_retransmissions(&mut self) {
        let now = Instant::now();
        let mut to_retry = Vec::new();
        for (client_addr, pending_messages) in &mut self.pending_acks {
            for (seq, (sent_time, message)) in pending_messages {
                if now.duration_since(*sent_time) > RETRY_TIMEOUT {
                    to_retry.push((*client_addr, *seq, message.clone()));
                }
            }
        }
        for (client_addr, seq, message) in to_retry {
            if let Some(pending_messages) = self.pending_acks.get_mut(&client_addr) {
                if let Some((ref mut sent_time, _)) = pending_messages.get_mut(&seq) {
                    *sent_time = now;
                    match self.socket.send_to(&message.bytes, client_addr) {
                        Ok(_) => {
                            println!("Resent message {:?} to client {:?}", seq, client_addr);
                        }
                        Err(e) => {
                            eprintln!(
                                "Failed to resend message {:?} to client {:?}: {}",
                                seq,
                                client_addr,
                                e
                            );
                        }
                    }
                }
            }
        }

        self.pending_acks.retain(|_, pending_messages| {
            pending_messages.retain(|_, (sent_time, _)| {
                now.duration_since(*sent_time) < RETRY_TIMEOUT * MAX_RETRIES
            });
            !pending_messages.is_empty()
        });
    }
    pub fn create_new_connection(&mut self, addr: &SocketAddr) {
        let new_id = ServerPlayerID(self.addr_to_player.len() as u8);
        self.addr_to_player.insert(*addr, new_id);
        self.pending_acks.insert(*addr, HashMap::new());
    }

    pub fn handle_message(&mut self, msg: ServerSideMessage, src: &SocketAddr) {
        if let Some(seq_num) = msg.seq_num {
            println!("msg arrived with seq num {}", seq_num);
            self.process_message(msg.msg, src);
            self.send_ack(SeqNum(seq_num), src);
        } else {
            self.process_message(msg.msg, src);
        }
    }

    fn process_message(&mut self, msg: NetworkMessage, src: &SocketAddr) {
        match msg {
            NetworkMessage::SendWorldState(data) => {
                println!("Processing world state update from {:?}", src);
            }
            NetworkMessage::SendPlayerInputs(inputs) => {
                println!("Processing player inputs from {:?}: {:?}", src, inputs);
            }
            NetworkMessage::GetServerPlayerIDs => {
                println!("Request for player IDS");
                let player_ids: Vec<u8> = self.addr_to_player
                    .values()
                    .into_iter()
                    .map(|v| v.0)
                    .filter(|id| self.addr_to_player[src].0 != *id)
                    .collect();
                self.send_reliable(NetworkMessage::SendServerPlayerIDs(player_ids), src);
            }
            NetworkMessage::ClientSideAck(seq_num) => {
                self.handle_ack(seq_num, src);
            }
            _ => {
                println!("Got some other message on server");
            }
            // Add other message types as needed
        }
    }
    pub fn handle_ack(&mut self, seq_num: SeqNum, src: &SocketAddr) {
        if let Some(pending_messages) = self.pending_acks.get_mut(src) {
            if pending_messages.remove(&seq_num).is_some() {
                println!("Acknowledged message {:?} from client {:?}", seq_num, src);
            } else {
                println!(
                    "Received acknowledgment for unknown message {:?} from client {:?}",
                    seq_num,
                    src
                );
            }
        } else {
            println!("Received acknowledgment from unknown client {:?}", src);
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
