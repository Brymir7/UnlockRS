use std::hash::Hash;
use std::net::{ SocketAddr, UdpSocket };
use std::collections::HashMap;
use std::process::exit;
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
    BufferedNetworkedPlayerInputs,
    SeqNum,
    SerializedMessageType,
    SerializedNetworkMessage,
    ServerPlayerID,
    SEQ_NUM_BYTE_POS,
};
mod type_impl;
mod types;
mod memory;
const MAX_RETRIES: u32 = 1;
const RETRY_TIMEOUT: Duration = Duration::from_millis(250);
struct Server {
    socket: UdpSocket,
    player_to_addr: [Option<SocketAddr>; (u8::MAX as usize) + 1],
    addr_to_player: HashMap<SocketAddr, ServerPlayerID>,
    pending_chunked_msgs: HashMap<SocketAddr, ChunkedMessageCollector>,
    connections: HashMap<SocketAddr, Vec<SocketAddr>>,
    msg_buffer: MsgBuffer,
    pending_acks: HashMap<SocketAddr, HashMap<SeqNum, (Instant, SerializedNetworkMessage)>>,
    sequence_number: u8,
    unack_input_seq_nums_to_frame: HashMap<SocketAddr, HashMap<SeqNum, u32>>,
    unack_input_buffer: HashMap<SocketAddr, BufferedNetworkedPlayerInputs>,
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
            unack_input_buffer: HashMap::new(),
            unack_input_seq_nums_to_frame: HashMap::new(),
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
                            //exit(1);
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
            pending_messages.retain(|seq, (sent_time, _)| {
                let resend = now.duration_since(*sent_time) < RETRY_TIMEOUT * MAX_RETRIES;
                if !resend {
                    println!("lost connection with {:?}", seq);
                }
                return resend;
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
        self.unack_input_buffer.insert(*addr, BufferedNetworkedPlayerInputs {
            buffered_inputs: Vec::new(),
        });
        self.unack_input_seq_nums_to_frame.insert(*addr, HashMap::new());
    }
    pub fn create_player_conn_from_to_host(
        &mut self,
        player1_addr: SocketAddr,
        player2_addr: SocketAddr
    ) {
        self.connections.entry(player1_addr).or_insert_with(Vec::new).push(player2_addr);
        self.connections.entry(player2_addr).or_insert_with(Vec::new).push(player1_addr);
        self.send_and_resend_until_ack(
            NetworkMessage::ServerRequestHostForWorldData,
            &player2_addr
        );
    }

    pub fn handle_message(&mut self, msg: DeserializedMessage, src: &SocketAddr) {
        if let Some(seq_num) = msg.seq_num {
            // println!("msg arrived with seq num {}", seq_num);
            self.process_message(msg.msg, src);
            self.send_ack(SeqNum(seq_num), src);
        } else {
            self.process_message(msg.msg, src);
        }
    }
    fn broadcast_reliable(&mut self, msg: NetworkMessage, src: &SocketAddr) {
        if let Some(connections) = self.connections.get(src) {
            let addresses: Vec<_> = connections.clone();
            for addr in addresses {
                self.send_and_resend_until_ack(msg.clone(), &addr);
                // todo this function shouldnt be called like this, it should be called once because not every client needs a different seq number
            } // every client alrdy has separate data due to hashmap socketaddr -> any information, so we can use the same seq num like below
        }
    }
    fn broadcast_inputs(&mut self, inputs: &BufferedNetworkedPlayerInputs, src: &SocketAddr) {
        let seq_num = SeqNum(self.sequence_number); // TODO refactor to not be able to use sequence number without increasing after
        self.sequence_number = self.sequence_number.wrapping_add(1);
        if let Some(inp_buffer) = self.unack_input_buffer.get_mut(src) {
            inp_buffer.insert_player_input(inputs.clone());
            if let Some(seq_num_to_frame) = self.unack_input_seq_nums_to_frame.get_mut(src) {
                seq_num_to_frame.insert(
                    seq_num,
                    inp_buffer.buffered_inputs
                        .last()
                        .expect("If we send sth it shouldnt be empty").frame
                );
            } else {
                println!(
                    "BUG seq_num_to_frame should always exist for a connection when inp buffer exists"
                );
            }
        } else {
            println!(
                "Unack input buffer doesn't exist for client, either he timed out or code is buggy"
            );
        }

        if let Some(connections) = self.connections.get(src) {
            let addresses: Vec<_> = connections.clone();
            let msg = NetworkMessage::ServerSentPlayerInputs(inputs.clone()).serialize(
                types::NetworkMessageType::SendOnceButReceiveAck(seq_num)
            );
            self.sequence_number = self.sequence_number.wrapping_add(1);
            match msg {
                SerializedMessageType::NonChunked(msg) => {
                    for addr in addresses {
                        self.socket.send_to(&msg.bytes, addr);
                    }
                }
                SerializedMessageType::Chunked(_) => panic!("Inputs should never be chunked"),
            }
        }
    }
    fn handle_player_input_ack(&mut self, seq_num: SeqNum, src: &SocketAddr) -> bool {
        if let Some(inp_buffer) = self.unack_input_buffer.get_mut(src) {
            if
                let Some(frame) = self.unack_input_seq_nums_to_frame
                    .get_mut(src)
                    .and_then(|seq_num_to_frame| seq_num_to_frame.remove(&seq_num))
            {
                inp_buffer.discard_acknowledged_frames(frame);
                return true;
            } else {
                println!("BUG: seq_num_to_frame should always exist when inp buffer exists");
            }
        } else {
            println!("Unack input buffer missing for client, possibly timeout or bug");
        }
        return false;
    }
    // fn send_bulk_until_ack(&mut self, msg. )
    fn process_message(&mut self, msg: NetworkMessage, src: &SocketAddr) {
        match msg {
            NetworkMessage::ClientSentWorld(data) => {
                // RELAY WORLD
                println!("received world state from client");
                self.broadcast_reliable(NetworkMessage::ServerSentWorld(data), src);
            }
            NetworkMessage::ClientSentPlayerInputs(inputs) => {
                // RELAY INPUTS
                // println!("Processing player inputs from {:?}: {:?}", src, inputs);
                self.broadcast_inputs(&inputs, src);
            }
            NetworkMessage::GetServerPlayerIDs => {
                // println!("Request for player IDS");
                let player_ids: Vec<u8> = self.addr_to_player
                    .iter()
                    .filter_map(|(addr, player)| {
                        if *addr != *src { Some(player.0) } else { None }
                    })
                    .collect();
                // println!("sent player ids {:?}", player_ids);
                self.send_and_resend_until_ack(
                    NetworkMessage::ServerSentPlayerIDs(player_ids),
                    src
                );
            }
            NetworkMessage::ClientSideAck(seq_num) => {
                self.handle_clients_ack(seq_num, src);
            }
            NetworkMessage::ClientConnectToOtherWorld(id) => {
                debug_assert!(id.0 != self.addr_to_player.get(src).unwrap().0);
                // println!(
                // "Connecting CALLER {:?} with TARGET {:?}",
                // id,
                // self.addr_to_player.get(src).unwrap()
                // );
                let other_player_addr = self.player_to_addr[id.0 as usize]
                    .clone()
                    .expect("Corrupt player to addr"); // TODO
                self.create_player_conn_from_to_host(*src, other_player_addr);
            }
            _ => {
                println!("Got some other message on server");
            }
            // Add other message types as needed
        }
    }
    pub fn handle_clients_ack(&mut self, seq_num: SeqNum, src: &SocketAddr) {
        let handled = self.handle_player_input_ack(seq_num, src);
        if handled {
            return;
        }
        if let Some(pending_messages) = self.pending_acks.get_mut(src) {
            if pending_messages.remove(&seq_num).is_some() {
                // println!("Acknowledged message {:?} from client {:?}", seq_num, src);
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
            types::NetworkMessageType::SendOnce
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

    pub fn send_and_resend_until_ack(&mut self, msg: NetworkMessage, dst: &SocketAddr) {
        let seq_num = SeqNum(self.sequence_number);
        //println!("trying to send message x to client {:?}", msg);

        let serialized_msg = msg.serialize(types::NetworkMessageType::ResendUntilAck(seq_num));
        match serialized_msg {
            SerializedMessageType::Chunked(chunks) => {
                for msg in chunks.bytes {
                    println!("sending chunked msg to client");
                    debug_assert!(msg[SEQ_NUM_BYTE_POS] == self.sequence_number);
                    // let msg_buffer = MsgBuffer(msg.clone().try_into().unwrap());
                    // debug_assert!(match msg_buffer.parse_on_client().expect("...") {
                    //     DeserializedMessageType::ChunkOfMessage(_) => true,
                    //     DeserializedMessageType::NonChunked(_) => false,
                    // });
                    if let Err(e) = self.socket.send_to(&msg, dst) {
                        eprintln!("Failed to send reliable message to {:?}: {}", dst, e);
                    }
                    self.pending_acks
                        .entry(*dst)
                        .or_insert_with(HashMap::new)
                        .insert(SeqNum(self.sequence_number), (
                            Instant::now(),
                            SerializedNetworkMessage { bytes: msg },
                        ));
                    self.sequence_number = self.sequence_number.wrapping_add(1);
                }
            }
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
}

fn main() -> std::io::Result<()> {
    let mut server = Server::new();
    loop {
        server.update();
    }
}
