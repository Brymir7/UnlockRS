use std::hash::Hash;
use std::net::{ SocketAddr, UdpSocket };
use std::collections::HashMap;
use std::time::{ Duration, Instant };
use macroquad::input;
use rand::seq;
use types::{
    ChunkOfMessage,
    ChunkedMessageCollector,
    DeserializedMessage,
    DeserializedMessageType,
    MsgBuffer,
    NetworkMessage,
    SeqNum,
    SerializedMessageType,
    SerializedNetworkMessage,
    ServerPlayerID,
};
mod type_impl;
mod types;
mod memory;
const MAX_RETRIES: u32 = 20;
const RETRY_TIMEOUT: Duration = Duration::from_millis(100);
struct Server {
    socket: UdpSocket,
    player_to_addr: [Option<SocketAddr>; (u8::MAX as usize) + 1],
    addr_to_player: HashMap<SocketAddr, ServerPlayerID>,
    pending_chunked_msgs: HashMap<SocketAddr, ChunkedMessageCollector>,
    connections: HashMap<SocketAddr, Vec<SocketAddr>>,
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
            player_to_addr: [None; (u8::MAX as usize) + 1],
            connections: HashMap::new(),
            pending_chunked_msgs: HashMap::new(),
            msg_buffer,
            pending_acks: HashMap::new(),
            sequence_number: 0,
        }
    }

    pub fn update(&mut self) {
        self.msg_buffer.clear(); // can rewrite to only read amt bytes form scoket
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

                if let Ok(server_side_msg) = msg {
                    match server_side_msg {
                        DeserializedMessageType::NonChunked(server_side_msg) => {
                            self.handle_message(server_side_msg, &src);
                        }
                        DeserializedMessageType::ChunkOfMessage(chunk) => {
                            self.send_ack(SeqNum(chunk.seq_num), &src);
                            if let Some(collector) = self.pending_chunked_msgs.get_mut(&src) {
                                collector.collect(chunk);
                                if let Some(msg) = collector.try_combine() {
                                    self.handle_message(msg, &src);
                                }
                            } else {
                                eprintln!("Couldnt find collector for addr {}", src);
                            }
                        }
                    }
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
        self.player_to_addr[new_id.0 as usize] = Some(*addr);
        self.pending_acks.insert(*addr, HashMap::new());
        self.pending_chunked_msgs.insert(*addr, ChunkedMessageCollector::default());
    }
    pub fn create_player_player_connection(
        &mut self,
        player1_addr: SocketAddr,
        player2_addr: SocketAddr
    ) {
        self.connections.entry(player1_addr).or_insert_with(Vec::new).push(player2_addr);
        self.connections.entry(player2_addr).or_insert_with(Vec::new).push(player1_addr);
    }

    pub fn handle_message(&mut self, msg: DeserializedMessage, src: &SocketAddr) {
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
            NetworkMessage::ClientSentWorld(data) => {
                // println!("Processing world state update from {:?}", src);
                // println!("first 10 bytes of data {:?}", data[0..10].to_vec());
            }
            NetworkMessage::ClientSentPlayerInputs(inputs) => {
                println!("Processing player inputs from {:?}: {:?}", src, inputs);
                if let Some(connections) = self.connections.get(src) {
                    for conn in connections {
                        self.send_unreliable(
                            NetworkMessage::ServerSentPlayerInputs(inputs.clone()),
                            conn
                        );
                    }
                }
            }
            NetworkMessage::GetServerPlayerIDs => {
                println!("Request for player IDS");
                let player_ids: Vec<u8> = self.addr_to_player
                    .iter()
                    .filter_map(|(addr, player)| {
                        if *addr != *src { Some(player.0) } else { None }
                    })
                    .collect();
                println!("sent player ids {:?}", player_ids);
                self.send_reliable(NetworkMessage::ServerSentPlayerIDs(player_ids), src);
            }
            NetworkMessage::ClientSideAck(seq_num) => {
                self.handle_ack(seq_num, src);
            }
            NetworkMessage::ClientConnectToOtherWorld(id) => {
                debug_assert!(id.0 != self.addr_to_player.get(src).unwrap().0);
                println!(
                    "Connecting CALLER {:?} with TARGET {:?}",
                    id,
                    self.addr_to_player.get(src).unwrap()
                );
                let other_player_addr = self.player_to_addr[id.0 as usize].clone().expect("Corrupt player to addr"); // TODO
                self.create_player_player_connection(*src, other_player_addr);
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
        match serialized_msg {
            SerializedMessageType::Chunked(_) => {
                panic!("Ack msg shouldnt need to be chunked");
            }
            SerializedMessageType::NonChunked(serialized_msg) => {
                if let Err(e) = self.socket.send_to(&serialized_msg.bytes, dst) {
                    eprintln!("Failed to send ACK to {:?}: {}", dst, e);
                }
            }
        }
    }
    pub fn send_reliable(&mut self, msg: NetworkMessage, dst: &SocketAddr) {
        let seq_num = SeqNum(self.sequence_number);
        let serialized_msg = msg.serialize(types::NetworkMessageType::Reliable(seq_num));
        match serialized_msg {
            SerializedMessageType::Chunked(chunks) => {}
            SerializedMessageType::NonChunked(serialized_msg) => {
                self.pending_acks
                    .entry(*dst)
                    .or_insert_with(HashMap::new)
                    .insert(seq_num, (Instant::now(), serialized_msg.clone()));
                self.sequence_number = self.sequence_number.wrapping_add(1);
                if let Err(e) = self.socket.send_to(&serialized_msg.bytes, dst) {
                    eprintln!("Failed to send reliable message to {:?}: {}", dst, e);
                }
            }
        }
    }

    pub fn send_unreliable(&self, msg: NetworkMessage, dst: &SocketAddr) {
        let serialized_msg = msg.serialize(types::NetworkMessageType::Unreliable);
        match serialized_msg {
            SerializedMessageType::Chunked(chunks) => {}
            SerializedMessageType::NonChunked(serialized_msg) => {
                if let Err(e) = self.socket.send_to(&serialized_msg.bytes, dst) {
                    eprintln!("Failed to send unreliable message to {:?}: {}", dst, e);
                }
            }
        }
    }
}

fn main() -> std::io::Result<()> {
    let mut server = Server::new();
    loop {
        server.update();
    }
}
