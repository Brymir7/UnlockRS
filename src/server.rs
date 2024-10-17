use std::char::MAX;
use std::f32::MIN;
use std::hash::Hash;
use std::net::{ SocketAddr, UdpSocket };
use std::collections::HashMap;
use std::process::exit;
use std::sync::mpsc;
use std::thread;
use std::time::{ Duration, Instant };
use macroquad::input;
use rand::{ seq, Rng };
use types::{
    BufferedNetworkedPlayerInputs,
    ChunkOfMessage,
    ChunkedMessageCollector,
    DeserializedMessage,
    DeserializedMessageType,
    LogConfig,
    Logger,
    MsgBuffer,
    NetworkMessage,
    SeqNum,
    SeqNumGenerator,
    SerializedMessageType,
    SerializedNetworkMessage,
    ServerPlayerID,
    MAX_UDP_PAYLOAD_LEN,
    SEQ_NUM_BYTE_POS,
};
mod type_impl;
mod types;
mod memory;

const MAX_RETRIES: u32 = 120;
const RETRY_TIMEOUT: Duration = Duration::from_millis(16);
const MIN_LATENCY: u64 = 20;
const MAX_LATENCY: u64 = 100;
const PACKET_LOSS: f32 = 0.02;

struct Server {
    socket: UdpSocket,
    player_to_addr: [Option<SocketAddr>; (u8::MAX as usize) + 1],
    addr_to_player: HashMap<SocketAddr, ServerPlayerID>,
    pending_chunked_msgs: HashMap<SocketAddr, ChunkedMessageCollector>,
    connections: HashMap<SocketAddr, Vec<SocketAddr>>,
    msg_buffer: MsgBuffer,
    pending_acks: HashMap<SocketAddr, HashMap<SeqNum, (Instant, SerializedNetworkMessage)>>,
    sequence_number: SeqNumGenerator,
    unack_input_seq_nums_to_frame: HashMap<SocketAddr, HashMap<SeqNum, u32>>,
    unack_input_buffer: HashMap<SocketAddr, BufferedNetworkedPlayerInputs>,
    logger: Logger,

    #[cfg(feature = "simulation_mode")]
    msg_sender: mpsc::Sender<(Vec<u8>, SocketAddr)>, // Only included if feature is enabled

    #[cfg(feature = "simulation_mode")]
    msg_rcv: mpsc::Receiver<(Vec<u8>, SocketAddr)>, // Only included if feature is enabled
}

impl Server {
    pub fn new() -> Self {
        let addr_to_player: HashMap<SocketAddr, ServerPlayerID> = HashMap::new();
        let socket = UdpSocket::bind("127.0.0.1:8080").expect("Server Failed to bind socket.");
        let msg_buffer: MsgBuffer = MsgBuffer::default();
        #[cfg(feature = "simulation_mode")]
        let msg_channel = mpsc::channel();
        Server {
            socket,
            addr_to_player,
            player_to_addr: [None; (u8::MAX as usize) + 1],
            connections: HashMap::new(),
            pending_chunked_msgs: HashMap::new(),
            msg_buffer,
            pending_acks: HashMap::new(),
            sequence_number: SeqNumGenerator {
                seq_num: SeqNum(0),
            },
            unack_input_buffer: HashMap::new(),
            unack_input_seq_nums_to_frame: HashMap::new(),
            logger: Logger::new(LogConfig::default()),

            #[cfg(feature = "simulation_mode")]
            msg_sender: msg_channel.0,
            #[cfg(feature = "simulation_mode")]
            msg_rcv: msg_channel.1,
        }
    }

    pub fn update(&mut self) {
        self.msg_buffer.clear();

        #[cfg(feature = "simulation_mode")]
        {
            let socket = self.socket.try_clone().expect("Failed to clone socket");
            let mut buffer = [0; MAX_UDP_PAYLOAD_LEN];
            let mut logger = self.logger.clone();
            let msg_sender = self.msg_sender.clone();
            match socket.recv_from(&mut buffer) {
                Ok((size, src)) => {
                    let mut rng = rand::thread_rng();
                    logger.debug_log_time("Received msg now!");
                    if rng.gen::<f32>() >= PACKET_LOSS {
                        let delay = rng.gen_range(MIN_LATENCY..=MAX_LATENCY);
                        thread::spawn(move || {
                            thread::sleep(Duration::from_millis(delay));
                            msg_sender
                                .send((buffer[..size].to_vec(), src))
                                .expect("Failed to send to channel");
                        });
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(e) => {
                    eprintln!("Error receiving data: {}", e);
                }
            }

            if let Ok((data, src)) = self.msg_rcv.try_recv() {
                self.msg_buffer.0[..data.len()].copy_from_slice(&data);

                if !self.addr_to_player.contains_key(&src) {
                    self.create_new_connection(&src);
                }

                let msg = self.msg_buffer.parse_on_server();
                if let Ok(server_side_msg) = msg {
                    match server_side_msg {
                        DeserializedMessageType::NonChunked(server_side_msg) => {
                            logger.debug_log_time("Handling msg now!");
                            self.handle_message(server_side_msg, &src);
                        }
                        DeserializedMessageType::ChunkOfMessage(chunk) => {
                            logger.debug_log_time("Handling msg now!");
                            self.send_ack(SeqNum(chunk.seq_num), &src);
                            if let Some(collector) = self.pending_chunked_msgs.get_mut(&src) {
                                collector.collect(chunk);
                                if let Some(msg) = collector.try_combine() {
                                    self.handle_message(msg, &src);
                                }
                            }
                        }
                    }
                }
            }
        }

        #[cfg(not(feature = "simulation_mode"))]
        {
            match self.socket.recv_from(&mut self.msg_buffer.0) {
                Ok((_, src)) => {
                    if !self.addr_to_player.contains_key(&src) {
                        self.create_new_connection(&src);
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
                                }
                            }
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => (),
                Err(e) => self.logger.error(format!("Error receiving data: {}", e)),
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
                            self.logger.message(
                                format!("Resent message {:?} to client {:?}", seq, client_addr)
                            );
                        }
                        Err(e) => {
                            self.logger.error(
                                format!(
                                    "Failed to resend message {:?} to client {:?}: {}",
                                    seq,
                                    client_addr,
                                    e
                                )
                            );
                        }
                    }
                }
            }
        }

        let _ = self.pending_acks.iter_mut().map(|(_, pending_messages)| {
            pending_messages.retain(|seq, (sent_time, _)| {
                let resend = now.duration_since(*sent_time) < RETRY_TIMEOUT * MAX_RETRIES;
                if !resend {
                    self.logger.connection(format!("Lost connection with {:?}", seq));
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
        self.logger.connection(format!("New connection established with {:?}", addr));
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
        self.logger.connection(
            format!("Created connection between {:?} and {:?}", player1_addr, player2_addr)
        );
    }

    pub fn handle_message(&mut self, msg: DeserializedMessage, src: &SocketAddr) {
        if let Some(seq_num) = msg.seq_num {
            self.logger.debug(format!("Message arrived with seq num {}", seq_num));
            self.process_message(msg.msg, src);
            self.send_ack(SeqNum(seq_num), src);
        } else {
            self.process_message(msg.msg, src);
        }
    }

    fn process_message(&mut self, msg: NetworkMessage, src: &SocketAddr) {
        match msg {
            NetworkMessage::ClientSentWorld(data) => {
                self.logger.world_state("Received world state from client");
                self.broadcast_reliable(NetworkMessage::ServerSentWorld(data), src);
            }
            NetworkMessage::ClientSentPlayerInputs(inputs) => {
                self.logger.player_input(
                    format!("Processing player inputs from {:?}: {:?}", src, inputs)
                );
                self.broadcast_inputs(&inputs, src);
            }
            NetworkMessage::GetServerPlayerIDs => {
                let player_ids: Vec<u8> = self.addr_to_player
                    .iter()
                    .filter_map(|(addr, player)| {
                        if *addr != *src { Some(player.0) } else { None }
                    })
                    .collect();
                self.logger.message(format!("Sending player IDs: {:?}", player_ids));
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
                let other_player_addr = self.player_to_addr[id.0 as usize]
                    .clone()
                    .expect("Corrupt player to addr");
                self.logger.connection("Client requesting connection");
                self.create_player_conn_from_to_host(*src, other_player_addr);
            }
            _ => {
                self.logger.debug("Received unhandled message type");
            }
        }
    }

    pub fn handle_clients_ack(&mut self, seq_num: SeqNum, src: &SocketAddr) {
        if let Some(pending_messages) = self.pending_acks.get_mut(src) {
            if pending_messages.remove(&seq_num).is_some() {
                self.logger.ack(
                    format!("Acknowledged message {:?} from client {:?}", seq_num, src)
                );
            } else {
                self.handle_player_input_ack(seq_num, src);
            }
        } else {
            self.logger.error(format!("Received acknowledgment from unknown client {:?}", src));
            self.logger.debug(format!("Pending acks: {:?}", self.pending_acks));
        }
    }

    fn send_ack(&mut self, seq_num: SeqNum, dst: &SocketAddr) {
        let serialized_msg = NetworkMessage::ServerSideAck(seq_num).serialize(
            types::NetworkMessageType::SendOnce
        );
        match serialized_msg {
            SerializedMessageType::Chunked(_) => {
                self.logger.error("ACK message shouldn't need to be chunked");
                panic!("Ack msg shouldnt need to be chunked");
            }
            SerializedMessageType::NonChunked(serialized_msg) => {
                if let Err(e) = self.socket.send_to(&serialized_msg.bytes, dst) {
                    self.logger.error(format!("Failed to send ACK to {:?}: {}", dst, e));
                }
            }
        }
    }

    pub fn send_and_resend_until_ack(&mut self, msg: NetworkMessage, dst: &SocketAddr) {
        self.logger.debug(format!("Sending message {:?} to client {:?}", msg, dst));
        let serialized_msg = msg.serialize(
            types::NetworkMessageType::ResendUntilAck(self.sequence_number.seq_num)
        );
        match serialized_msg {
            SerializedMessageType::Chunked(chunks) => {
                for msg in chunks.bytes {
                    let seq_num = self.sequence_number.get_seq_num();
                    self.logger.message("Sending chunked message to client");
                    debug_assert!(
                        u16::from_le_bytes([msg[SEQ_NUM_BYTE_POS], msg[SEQ_NUM_BYTE_POS + 1]]) ==
                            seq_num.0
                    );
                    if let Err(e) = self.socket.send_to(&msg, dst) {
                        self.logger.error(
                            format!("Failed to send reliable message to {:?}: {}", dst, e)
                        );
                    }
                    self.pending_acks
                        .entry(*dst)
                        .or_insert_with(HashMap::new)
                        .insert(seq_num, (Instant::now(), SerializedNetworkMessage { bytes: msg }));
                }
            }
            SerializedMessageType::NonChunked(serialized_msg) => {
                let seq_num = self.sequence_number.get_seq_num();
                self.pending_acks
                    .entry(*dst)
                    .or_insert_with(HashMap::new)
                    .insert(seq_num, (Instant::now(), serialized_msg.clone()));
                if let Err(e) = self.socket.send_to(&serialized_msg.bytes, dst) {
                    self.logger.error(
                        format!("Failed to send reliable message to {:?}: {}", dst, e)
                    );
                }
            }
        }
    }

    fn broadcast_reliable(&mut self, msg: NetworkMessage, src: &SocketAddr) {
        if let Some(connections) = self.connections.get(src) {
            let addresses: Vec<_> = connections.clone();
            for addr in addresses {
                self.send_and_resend_until_ack(msg.clone(), &addr);
            }
        }
    }

    fn broadcast_inputs(&mut self, inputs: &BufferedNetworkedPlayerInputs, src: &SocketAddr) {
        let seq_num = self.sequence_number.get_seq_num();
        if let Some(connections) = self.connections.get(src) {
            let msg = NetworkMessage::ServerSentPlayerInputs(inputs.clone()).serialize(
                types::NetworkMessageType::SendOnceButReceiveAck(seq_num)
            );

            match msg {
                SerializedMessageType::NonChunked(msg) => {
                    for addr in connections.clone() {
                        if let Some(inp_buffer) = self.unack_input_buffer.get_mut(&addr) {
                            inp_buffer.bulk_insert_player_input(inputs.clone());
                            if
                                let Some(seq_num_to_frame) =
                                    self.unack_input_seq_nums_to_frame.get_mut(&addr)
                            {
                                seq_num_to_frame.insert(
                                    seq_num,
                                    inp_buffer.buffered_inputs
                                        .last()
                                        .expect("If we send sth it shouldnt be empty").frame
                                );

                                #[cfg(feature = "simulation_mode")]
                                {
                                    let socket = self.socket
                                        .try_clone()
                                        .expect("Failed to clone socket");
                                    let msg_bytes = msg.bytes.clone();
                                    let dst = addr;
                                    let mut logger = self.logger.clone();
                                    logger.debug_log_time(format!("Simulating latency message"));
                                    thread::spawn(move || {
                                        let mut rng = rand::thread_rng();
                                        if rng.gen::<f32>() < PACKET_LOSS {
                                            return;
                                        }
                                        thread::sleep(
                                            Duration::from_millis(
                                                rng.gen_range(MIN_LATENCY..=MAX_LATENCY)
                                            )
                                        );
                                        logger.debug_log_time(
                                            format!("Simulating latency message")
                                        );
                                        if let Err(e) = socket.send_to(&msg_bytes, dst) {
                                            eprintln!("Failed to send input message: {}", e);
                                        }
                                    });
                                }

                                #[cfg(not(feature = "simulation_mode"))]
                                {
                                    if let Err(e) = self.socket.send_to(&msg.bytes, addr) {
                                        self.logger.error(
                                            format!("Failed to send input message: {}", e)
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
                SerializedMessageType::Chunked(_) => {
                    self.logger.error("Inputs should never be chunked");
                    panic!("Inputs should never be chunked");
                }
            }
        }
    }

    fn handle_player_input_ack(&mut self, seq_num: SeqNum, src: &SocketAddr) {
        if let Some(inp_buffer) = self.unack_input_buffer.get_mut(src) {
            if let Some(seq_num_to_frame) = self.unack_input_seq_nums_to_frame.get_mut(src) {
                if let Some(frame) = seq_num_to_frame.remove(&seq_num) {
                    inp_buffer.discard_acknowledged_frames(frame);
                }
            } else {
                self.logger.error(
                    "BUG: seq_num_to_frame should always exist when inp buffer exists"
                );
            }
        } else {
            self.logger.error("Unack input buffer missing for client, possibly timeout or bug");
        }
    }
}

fn main() -> std::io::Result<()> {
    let mut server = Server::new();
    server.logger.message("Server started on 127.0.0.1:8080");
    loop {
        server.update();
    }
}
