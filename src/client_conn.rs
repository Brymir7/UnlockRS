use core::panic;
use std::{
    collections::{ HashMap, HashSet },
    net::UdpSocket,
    sync::{ mpsc, Arc, Mutex },
    thread,
    time::{ Duration, Instant },
};

use rand::seq;
#[cfg(feature = "simulation_mode")]
use simulation::{ PACKET_LOSS_PERCENTAGE, rng_gen_range };
#[cfg(feature = "simulation_mode")]
mod simulation {
    use std::{ ops::Range, time::Duration };
    use rand::Rng;
    pub fn rng_gen_range(range: Range<f32>) -> f32 {
        let mut rng = rand::thread_rng();
        rng.gen_range(range)
    }
    pub const PACKET_LOSS_PERCENTAGE: f32 = 75.0;
    pub const LATENCY_MS: Duration = Duration::from_millis(100);
}
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
    sequence_number: u8,
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
                sequence_number: 0,
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
        let msg_sender = self.network_msg_sender.clone();
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
                                    let _ = msg_sender.send(request.msg);
                                }
                                crate::types::DeserializedMessageType::ChunkOfMessage(chunk) => {
                                    let _ = ack_sender.send(SeqNum(chunk.seq_num));
                                    let mut chunk_collector = chunk_collector.lock().unwrap();
                                    chunk_collector.collect(chunk);
                                    println!("Collected chunk");
                                    if let Some(msg) = chunk_collector.try_combine() {
                                        let _ = msg_sender.send(msg.msg);
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
    pub fn handle_ack(&mut self, acked_seq_num: SeqNum) {
        self.pending_acks.remove(&acked_seq_num);
    }
    // pub fn send_unreliable(&self, request: &NetworkMessage) -> Result<(), std::io::Error> {
    //     let serialized_message = request.serialize(crate::types::NetworkMessageType::SendOnce);
    //     #[cfg(feature = "simulation_mode")]
    //     {
    //         if rng_gen_range(0.0..100.0) < PACKET_LOSS_PERCENTAGE {
    //             LOGGER.log_simulated_packet_loss(self.sequence_number);
    //             return Ok(());
    //         }
    //     }
    //     match serialized_message {
    //         crate::types::SerializedMessageType::NonChunked(serialized_message) => {
    //             self.socket.send(&serialized_message.bytes)?;
    //             Ok(())
    //         }
    //         crate::types::SerializedMessageType::Chunked(_) => {
    //             panic!("Cannot send unreliable in chunks rn");
    //         }
    //     }
    // }

    pub fn send_reliable(&mut self, request: &NetworkMessage) -> Result<(), std::io::Error> {
        let serialized_message = request.serialize(
            crate::types::NetworkMessageType::ResendUntilAck(SeqNum(self.sequence_number))
        );
        match serialized_message {
            crate::types::SerializedMessageType::Chunked(chunks) => {
                for msg in chunks.bytes {
                    #[cfg(feature = "simulation_mode")]
                    {
                        if rng_gen_range(0.0..100.0) < PACKET_LOSS_PERCENTAGE {
                            self.pending_acks.insert(SeqNum(self.sequence_number), (
                                Instant::now(),
                                SerializedNetworkMessage { bytes: msg.clone() },
                            ));
                            self.sequence_number = self.sequence_number.wrapping_add(1);
                            LOGGER.log_simulated_packet_loss(self.sequence_number);
                            continue;
                        }
                    }
                    debug_assert!(msg[SEQ_NUM_BYTE_POS] == self.sequence_number);
                    self.socket.send(&msg)?;
                    self.pending_acks.insert(SeqNum(self.sequence_number), (
                        Instant::now(),
                        SerializedNetworkMessage { bytes: msg },
                    ));
                    LOGGER.log_sent_packet(self.sequence_number);
                    self.sequence_number = self.sequence_number.wrapping_add(1);
                }
                Ok(())
            }
            crate::types::SerializedMessageType::NonChunked(serialized_message) => {
                #[cfg(feature = "simulation_mode")]
                {
                    if rng_gen_range(0.0..100.0) < PACKET_LOSS_PERCENTAGE {
                        self.pending_acks.insert(SeqNum(self.sequence_number), (
                            Instant::now(),
                            serialized_message.clone(),
                        ));
                        self.sequence_number = self.sequence_number.wrapping_add(1);
                        LOGGER.log_simulated_packet_loss(self.sequence_number);
                        return Ok(());
                    }
                }
                self.socket.send(&serialized_message.bytes)?;
                self.pending_acks.insert(SeqNum(self.sequence_number), (
                    Instant::now(),
                    serialized_message,
                ));
                self.sequence_number = self.sequence_number.wrapping_add(1);
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
        if
            self.unack_input_buffer.buffered_inputs.len() * 5 < MAX_UDP_PAYLOAD_DATA_LENGTH - 1 // 5 bytes 4 for frame, 1 for input, and 1 start bit for length of vec
        {
            return Err(SendInputsError::Disconnected);
        }
        self.unack_input_buffer.buffered_inputs.push(inputs.clone());
        self.unack_input_seq_nums_to_frame.insert(SeqNum(self.sequence_number), inputs.frame);

        let request = NetworkMessage::ClientSentPlayerInputs(
            self.unack_input_buffer.clone()
        ).serialize(NetworkMessageType::SendOnceButReceiveAck(SeqNum(self.sequence_number)));

        #[cfg(feature = "simulation_mode")]
        {
            if rng_gen_range(0.0..100.0) < PACKET_LOSS_PERCENTAGE {
                LOGGER.log_simulated_packet_loss(self.sequence_number);
                return Ok(());
            }
        }
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
