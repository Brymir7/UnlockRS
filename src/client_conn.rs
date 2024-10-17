use core::panic;
use std::{
    collections::{ HashMap, HashSet },
    net::UdpSocket,
    sync::{ mpsc, Arc, Mutex },
    thread::{ self, sleep },
    time::{ Duration, Instant },
};

const LOGGER: NetworkLogger = NetworkLogger { log: false };
use crate::types::{
    BufferedNetworkedPlayerInputs,
    ChunkedMessageCollector,
    DeserializedMessage,
    GameMessage,
    GameRequestToNetwork,
    MsgBuffer,
    NetworkLogger,
    NetworkMessage,
    NetworkMessageType,
    NetworkedPlayerInput,
    PlayerInput,
    SendInputsError,
    SeqNum,
    SeqNumGenerator,
    SerializedNetworkMessage,
    ServerPlayerID,
    MAX_UDP_PAYLOAD_DATA_LENGTH,
    MAX_UDP_PAYLOAD_LEN,
    SEQ_NUM_BYTE_POS,
};

const MAX_RETRIES: u32 = 8;
const RETRY_TIMEOUT: Duration = Duration::from_millis(250);
pub struct ConnectionServer {
    socket: Arc<UdpSocket>,
    sequence_number: SeqNumGenerator,
    pending_acks: HashMap<SeqNum, (Instant, SerializedNetworkMessage)>,
    server_msg_sender: mpsc::Sender<NetworkMessage>,
    client_request_receiver: mpsc::Receiver<GameRequestToNetwork>,
    ack_sender: mpsc::Sender<SeqNum>,
    ack_receiver: mpsc::Receiver<SeqNum>,
    network_msg_receiver: mpsc::Receiver<NetworkMessage>,
    network_msg_sender: mpsc::Sender<NetworkMessage>,
    chunked_msg_collector: Arc<Mutex<ChunkedMessageCollector>>,
    unack_input_buffer: BufferedNetworkedPlayerInputs,
    unack_input_seq_nums_to_frame: HashMap<SeqNum, u32>, // Hashmaps from seq_num to u32 could also be rewritten as vecs / depending on seq_num_size as static arrays
}

impl ConnectionServer {
    pub fn new() -> Result<
        (
            Arc<Mutex<ConnectionServer>>,
            mpsc::Sender<GameRequestToNetwork>,
            mpsc::Receiver<NetworkMessage>,
        ),
        std::io::Error
    > {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0")?);
        socket.connect("127.0.0.1:8080")?;

        let (response_sender, response_receiver) = mpsc::channel();
        let (request_sender, request_receiver) = mpsc::channel();
        let (ack_sender, ack_receiver) = mpsc::channel();
        let (network_msg_sender, network_msg_receiver) = mpsc::channel();
        let connection_server = Arc::new(
            Mutex::new(ConnectionServer {
                socket,
                sequence_number: SeqNumGenerator {
                    seq_num: SeqNum(0),
                },
                pending_acks: HashMap::new(),
                server_msg_sender: response_sender,
                client_request_receiver: request_receiver,
                ack_sender,
                ack_receiver,
                network_msg_sender,
                network_msg_receiver,
                chunked_msg_collector: Arc::new(Mutex::new(ChunkedMessageCollector::default())),
                unack_input_buffer: BufferedNetworkedPlayerInputs {
                    buffered_inputs: Vec::new(),
                },
                unack_input_seq_nums_to_frame: HashMap::new(),
            })
        );

        Ok((connection_server, request_sender, response_receiver))
    }
    pub fn start(server: Arc<Mutex<ConnectionServer>>) {
        thread::spawn(move || {
            server.lock().unwrap().run();
        });
    }

    pub fn run(&mut self) {
        let receive_socket = Arc::clone(&self.socket);
        let ack_sender = self.ack_sender.clone();
        let chunk_collector = Arc::clone(&self.chunked_msg_collector);
        let parsed_network_msg_sender = self.network_msg_sender.clone();
        let receive_thread = thread::spawn(move || {
            let mut buffer = MsgBuffer::default();
            loop {
                buffer.clear();
                match receive_socket.recv(&mut buffer.0) {
                    Ok(amt) if amt > 0 => {
                        if let Ok(request) = buffer.parse_on_client() {
                            match request {
                                crate::types::DeserializedMessageType::NonChunked(request) => {
                                    debug_assert!(
                                        (request.seq_num.is_some() && request.reliable) ||
                                            (!request.reliable && request.seq_num.is_none())
                                    );
                                    if let Some(seq_num) = request.seq_num {
                                        let _ = ack_sender.send(SeqNum(seq_num));
                                    }
                                    let _ = parsed_network_msg_sender.send(request.msg);
                                }
                                crate::types::DeserializedMessageType::ChunkOfMessage(chunk) => {
                                    let _ = ack_sender.send(SeqNum(chunk.seq_num));
                                    let mut chunk_collector = chunk_collector.lock().unwrap();
                                    chunk_collector.collect(chunk);
                                    println!("Collected chunk");
                                    if let Some(msg) = chunk_collector.try_combine() {
                                        let _ = parsed_network_msg_sender.send(msg.msg);
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to receive: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        });

        loop {
            if let Ok(ack) = self.ack_receiver.try_recv() {
                self.send_ack(ack);
            }
            if let Ok(msg) = self.network_msg_receiver.try_recv() {
                match msg {
                    NetworkMessage::ServerSentWorld(data) => {
                        println!("server sent world arrived");
                        let _ = self.server_msg_sender.send(NetworkMessage::ServerSentWorld(data));
                    }
                    NetworkMessage::ServerSentPlayerInputs(inputs) => {
                        // debug_assert!(
                        //     inputs.buffered_inputs.windows(2).all(|w| w[0].frame + 1 == w[1].frame),
                        //     "Frames are not in order or there are gaps"
                        // );

                        let _ = self.server_msg_sender.send(
                            NetworkMessage::ServerSentPlayerInputs(inputs)
                        );
                    }
                    NetworkMessage::ServerSideAck(acked_seq_num) => {
                        self.handle_ack(acked_seq_num);
                        LOGGER.log_received_ack(acked_seq_num.0);
                    }
                    NetworkMessage::ServerSentPlayerIDs(ids) => {
                        let _ = self.server_msg_sender.send(
                            NetworkMessage::ServerSentPlayerIDs(ids)
                        );
                    }
                    NetworkMessage::ServerRequestHostForWorldData => {
                        let _ = self.server_msg_sender.send(
                            NetworkMessage::ServerRequestHostForWorldData
                        );
                    }
                    _ => {}
                }
            }
            match self.client_request_receiver.try_recv() {
                Ok(request) => {
                    match request {
                        GameRequestToNetwork::DirectRequest(network_msg) => {
                            match network_msg {
                                NetworkMessage::GetOwnServerPlayerID => {
                                    todo!();
                                }
                                NetworkMessage::GetServerPlayerIDs => {
                                    if let Err(e) = self.get_available_player_worlds() {
                                        eprintln!("Error getting available player worlds: {}", e);
                                    }
                                }
                                NetworkMessage::ClientSentWorld(sim_mem) => {
                                    if let Err(e) = self.send_player_world_state(sim_mem) {
                                        eprintln!("Error sending player world state: {}", e);
                                    }
                                }

                                NetworkMessage::ClientConnectToOtherWorld(id) => {
                                    if let Err(e) = self.connect_to_other_world(id) {
                                        eprintln!("Error connecting to other world: {}", e);
                                    }
                                }
                                NetworkMessage::ClientSentPlayerInputs(_) => {
                                    panic!(
                                        "Client cannot send buffered inputs, network takes caree of this"
                                    );
                                }
                                _ => {
                                    panic!(
                                        "Tried to run server side NetworkMessage on client {:?}",
                                        network_msg
                                    );
                                }
                            }
                        }
                        GameRequestToNetwork::IndirectRequest(game_msg) => {
                            match game_msg {
                                GameMessage::ClientSentPlayerInputs(inp) => {
                                    self.send_player_inputs(inp);
                                }
                            }
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    // No message received, continue with other operations
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    // Channel has been disconnected, exit the loop
                    break;
                }
            }

            self.handle_retransmissions();
        }
        receive_thread.join().unwrap();
    }
    pub fn handle_server_input_ack(&mut self, seq_num: SeqNum) -> bool {
        if let Some(frame) = self.unack_input_seq_nums_to_frame.remove(&seq_num) {
            self.unack_input_buffer.discard_acknowledged_frames(frame);
            return true;
        }
        return false;
    }
    pub fn handle_ack(&mut self, acked_seq_num: SeqNum) {
        let handled = self.handle_server_input_ack(acked_seq_num);
        if handled {
            return;
        }
        self.pending_acks.remove(&acked_seq_num);
    }

    pub fn send_reliable(&mut self, request: &NetworkMessage) -> Result<(), std::io::Error> {
        let seq_num = self.sequence_number.get_seq_num();
        let serialized_message = request.serialize(
            crate::types::NetworkMessageType::ResendUntilAck(seq_num)
        );
        match serialized_message {
            crate::types::SerializedMessageType::Chunked(chunks) => {
                for msg in chunks.bytes {
                    debug_assert!(
                        u16::from_le_bytes([msg[SEQ_NUM_BYTE_POS], msg[SEQ_NUM_BYTE_POS + 1]]) ==
                            seq_num.0
                    );
                    self.socket.send(&msg)?;
                    self.pending_acks.insert(seq_num, (
                        Instant::now(),
                        SerializedNetworkMessage { bytes: msg },
                    ));
                    LOGGER.log_sent_packet(seq_num.0);
                }
                Ok(())
            }
            crate::types::SerializedMessageType::NonChunked(serialized_message) => {
                self.socket.send(&serialized_message.bytes)?;
                self.pending_acks.insert(seq_num, (Instant::now(), serialized_message));
                Ok(())
            }
        }
    }

    fn send_ack(&self, seq_num: SeqNum) {
        let ack_message = NetworkMessage::ClientSideAck(seq_num).serialize(
            NetworkMessageType::ResendUntilAck(seq_num)
        );
        match ack_message {
            crate::types::SerializedMessageType::NonChunked(serialized_msg) => {
                if let Err(e) = self.socket.send(&serialized_msg.bytes) {
                    eprintln!("Failed to send ACK to server: {}", e);
                }
            }
            crate::types::SerializedMessageType::Chunked(_) => {
                panic!("ack shouldnt be chunked");
            }
        }
    }
    fn handle_retransmissions(&mut self) {
        let now = Instant::now();
        let mut to_retry = Vec::new();

        {
            for (seq, (sent_time, request)) in self.pending_acks.iter() {
                if now.duration_since(*sent_time) > RETRY_TIMEOUT {
                    to_retry.push((*seq, request.clone()));
                }
            }
            self.pending_acks.retain(|_, (sent_time, _)| {
                now.duration_since(*sent_time) < RETRY_TIMEOUT * MAX_RETRIES
            });
        }

        for (seq, request) in to_retry {
            if let Some((ref mut sent_time, _)) = self.pending_acks.get_mut(&seq) {
                *sent_time = now;
                LOGGER.log_sent_retransmission(seq.0);
                if let Err(e) = self.socket.send(&request.bytes) {
                    eprintln!("Failed to resend message {:?}: {}", seq, e);
                }
            }
        }
    }

    fn send_player_world_state(&mut self, sim_mem: Vec<u8>) -> Result<(), std::io::Error> {
        let request = NetworkMessage::ClientSentWorld(sim_mem.clone()); // TODO REWRITE THIS TO JUST USE REQUEST
        self.send_reliable(&request)
    }

    fn get_available_player_worlds(&mut self) -> Result<(), std::io::Error> {
        let request = NetworkMessage::GetServerPlayerIDs;
        self.send_reliable(&request)
    }
    fn connect_to_other_world(&mut self, id: ServerPlayerID) -> Result<(), std::io::Error> {
        let request = NetworkMessage::ClientConnectToOtherWorld(id);
        self.send_reliable(&request)
    }
    fn send_player_inputs(&mut self, inputs: NetworkedPlayerInput) -> Result<(), SendInputsError> {
        // let request = NetworkMessage::ClientSentPlayerInputs(inputs);
        // if they have the same length then we couldnt send inputs for multiple seconds, so we stop sending and disconnect
        let seq_num = self.sequence_number.get_seq_num();
        if
            (self.unack_input_buffer.buffered_inputs.len() + 1) * 5 >
            MAX_UDP_PAYLOAD_DATA_LENGTH - 1 // if new input would overflow;  5 bytes 4 for frame, 1 for input, and 1 start bit for length of vec
        {
            println!("No player to player connection, DISCONNECTED");
            self.unack_input_buffer.buffered_inputs.swap_remove(0); // remove first
            return Err(SendInputsError::Disconnected);
        }
        println!("client sent sth for frame {:?}", inputs.frame);
        self.unack_input_buffer.insert_player_input(inputs.clone());
        self.unack_input_seq_nums_to_frame.insert(seq_num, inputs.frame);
        // debug_assert!(
        //     self.unack_input_buffer.buffered_inputs.windows(2).all(|i| i[0].frame + 1 == i[1].frame)
        // );
        let request = NetworkMessage::ClientSentPlayerInputs(
            self.unack_input_buffer.clone()
        ).serialize(NetworkMessageType::SendOnceButReceiveAck(seq_num));

        match request {
            crate::types::SerializedMessageType::NonChunked(request) => {
                let res = self.socket.send(&request.bytes);
                match res {
                    Ok(_) => {
                        return Ok(());
                    }
                    Err(e) => {
                        return Err(SendInputsError::IO(e));
                    }
                }
            }
            _ => panic!("Invalid type for send inputs request"),
        }
    }
}
