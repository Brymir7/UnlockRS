use std::{
    collections::HashMap,
    net::UdpSocket,
    sync::{ mpsc, Arc, Mutex },
    thread,
    time::{ Duration, Instant },
};

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
    MsgBuffer,
    NetworkLogger,
    NetworkMessage,
    NetworkMessageType,
    NetworkedPlayerInputs,
    PlayerInput,
    SeqNum,
    SerializedNetworkMessage,
    ServerPlayerID,
    SEQ_NUM_BYTE_POS,
};

const MAX_RETRIES: u32 = 8;
const RETRY_TIMEOUT: Duration = Duration::from_millis(250);
pub struct ConnectionServer {
    socket: Arc<UdpSocket>,
    sequence_number: u8,
    pending_acks: Arc<Mutex<HashMap<SeqNum, (Instant, SerializedNetworkMessage)>>>,
    response_sender: mpsc::Sender<NetworkMessage>,
    request_receiver: mpsc::Receiver<NetworkMessage>,
    ack_sender: mpsc::Sender<SeqNum>,
    ack_receiver: mpsc::Receiver<SeqNum>,
}

impl ConnectionServer {
    pub fn new() -> Result<
        (
            Arc<Mutex<ConnectionServer>>,
            mpsc::Sender<NetworkMessage>,
            mpsc::Receiver<NetworkMessage>,
        ),
        std::io::Error
    > {
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0")?);
        socket.connect("127.0.0.1:8080")?;

        let (response_sender, response_receiver) = mpsc::channel();
        let (request_sender, request_receiver) = mpsc::channel();
        let (ack_sender, ack_receiver) = mpsc::channel();

        let connection_server = Arc::new(
            Mutex::new(ConnectionServer {
                socket,
                sequence_number: 0,
                pending_acks: Arc::new(Mutex::new(HashMap::new())),
                response_sender,
                request_receiver,
                ack_sender,
                ack_receiver,
            })
        );

        Ok((connection_server, request_sender, response_receiver))
    }
    pub fn start(server: Arc<Mutex<ConnectionServer>>) {
        let server_clone = Arc::clone(&server);
        thread::spawn(move || {
            server_clone.lock().unwrap().run();
        });
    }

    pub fn run(&mut self) {
        let receive_socket = Arc::clone(&self.socket);
        let response_sender = self.response_sender.clone();
        let ack_sender = self.ack_sender.clone();
        let pending_acks = Arc::clone(&self.pending_acks);

        let receive_thread = thread::spawn(move || {
            let mut buffer = MsgBuffer::default();
            loop {
                buffer.clear();
                match receive_socket.recv(&mut buffer.0) {
                    Ok(amt) if amt > 0 => {
                        if let Ok(request) = buffer.parse_on_client() {
                            if let Some(seq_num) = request.seq_num {
                                let _ = ack_sender.send(SeqNum(seq_num));
                            }
                            match request.msg {
                                NetworkMessage::ServerSentWorld(data) => {}
                                NetworkMessage::ServerSentPlayerInputs(inputs) => {
                                    let _ = response_sender.send(
                                        NetworkMessage::ServerSentPlayerInputs(inputs)
                                    );
                                }
                                NetworkMessage::ServerSideAck(type_of_ack) => {
                                    pending_acks.lock().unwrap().remove(&type_of_ack);
                                    LOGGER.log_received_ack(type_of_ack.0);
                                }
                                NetworkMessage::ServerSentPlayerIDs(ids) => {
                                    let _ = response_sender.send(
                                        NetworkMessage::ServerSentPlayerIDs(ids)
                                    );
                                }
                                _ => {}
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
            match self.request_receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(request) => {
                    match request {
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
                        NetworkMessage::ClientSentPlayerInputs(inputs) => {
                            if let Err(e) = self.send_player_inputs(inputs) {
                                eprintln!("Error sending player inputs: {}", e);
                            }
                        }
                        NetworkMessage::ClientConnectToOtherWorld(id) => {
                            if let Err(e) = self.connect_to_other_world(id) {
                                eprintln!("Error connecting to other world: {}", e);
                            }
                        }
                        _ => {
                            panic!(
                                "Tried to run server side NetworkMessage on client {:?}",
                                request
                            );
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // No message received, continue with other operations
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    // Channel has been disconnected, exit the loop
                    break;
                }
            }

            self.handle_retransmissions();
        }

        receive_thread.join().unwrap();
    }
    pub fn send_unreliable(&self, request: &NetworkMessage) -> Result<(), std::io::Error> {
        let serialized_message = request.serialize(crate::types::NetworkMessageType::Unreliable);
        #[cfg(feature = "simulation_mode")]
        {
            if rng_gen_range(0.0..100.0) < PACKET_LOSS_PERCENTAGE {
                LOGGER.log_simulated_packet_loss(self.sequence_number);
                return Ok(());
            }
        }
        match serialized_message {
            crate::types::SerializedMessageType::NonChunked(serialized_message) => {
                self.socket.send(&serialized_message.bytes)?;
                Ok(())
            }
            crate::types::SerializedMessageType::Chunked(_) => {
                panic!("Cannot send unreliable in chunks rn");
            }
        }
    }

    pub fn send_reliable(&mut self, request: &NetworkMessage) -> Result<(), std::io::Error> {
        let serialized_message = request.serialize(
            crate::types::NetworkMessageType::Reliable(SeqNum(self.sequence_number))
        );
        match serialized_message {
            crate::types::SerializedMessageType::Chunked(chunks) => {
                for msg in chunks.bytes {
                    #[cfg(feature = "simulation_mode")]
                    {
                        if rng_gen_range(0.0..100.0) < PACKET_LOSS_PERCENTAGE {
                            self.pending_acks
                                .lock()
                                .unwrap()
                                .insert(SeqNum(self.sequence_number), (
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
                    self.pending_acks
                        .lock()
                        .unwrap()
                        .insert(SeqNum(self.sequence_number), (
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
                        self.pending_acks
                            .lock()
                            .unwrap()
                            .insert(SeqNum(self.sequence_number), (
                                Instant::now(),
                                serialized_message.clone(),
                            ));
                        self.sequence_number = self.sequence_number.wrapping_add(1);
                        LOGGER.log_simulated_packet_loss(self.sequence_number);
                        return Ok(());
                    }
                }
                self.socket.send(&serialized_message.bytes)?;
                self.pending_acks
                    .lock()
                    .unwrap()
                    .insert(SeqNum(self.sequence_number), (Instant::now(), serialized_message));
                self.sequence_number = self.sequence_number.wrapping_add(1);
                Ok(())
            }
        }
    }

    fn send_ack(&self, seq_num: SeqNum) {
        let ack_message = NetworkMessage::ClientSideAck(seq_num);
        let serialized_msg = ack_message.serialize(NetworkMessageType::Unreliable);
        match serialized_msg {
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
    fn handle_ack(&self, type_of_ack: SeqNum) {
        LOGGER.log_received_ack(type_of_ack.0);
        let mut pending_acks = self.pending_acks.lock().unwrap();
        pending_acks.remove(&type_of_ack);
        LOGGER.log_pending_acks(pending_acks.keys().cloned().collect());
    }
    fn handle_retransmissions(&mut self) {
        let now = Instant::now();
        let mut to_retry = Vec::new();

        {
            let mut pending_acks = self.pending_acks.lock().unwrap();
            for (seq, (sent_time, request)) in pending_acks.iter() {
                if now.duration_since(*sent_time) > RETRY_TIMEOUT {
                    to_retry.push((*seq, request.clone()));
                }
            }
            pending_acks.retain(|_, (sent_time, _)| {
                now.duration_since(*sent_time) < RETRY_TIMEOUT * MAX_RETRIES
            });
        }

        for (seq, request) in to_retry {
            if let Some((ref mut sent_time, _)) = self.pending_acks.lock().unwrap().get_mut(&seq) {
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
    fn send_player_inputs(&mut self, inputs: NetworkedPlayerInputs) -> Result<(), std::io::Error> {
        if inputs.inputs.len() == 0 {
            return Ok(());
        }
        let request = NetworkMessage::ClientSentPlayerInputs(inputs);
        self.send_unreliable(&request)
    }
}
