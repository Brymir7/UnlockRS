use std::{ collections::HashMap, net::UdpSocket, thread, time::{ Duration, Instant } };

#[cfg(feature = "simulation_mode")]
use simulation::{ LATENCY_MS, PACKET_LOSS_PERCENTAGE, rng_gen_range };
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
use crate::{
    memory::PageAllocator,
    types::{
        ChunkedSerializedNetworkMessage,
        MsgBuffer,
        NetworkLogger,
        NetworkMessage,
        NetworkMessageType,
        PlayerInput,
        SeqNum,
        SerializedNetworkMessage,
        ServerPlayerID,
        Simulation,
        SEQ_NUM_BYTE_POS,
    },
};

const MAX_RETRIES: u32 = 8;
const RETRY_TIMEOUT: Duration = Duration::from_millis(250);
use tokio::sync::mpsc;
pub struct ConnectionServer {
    socket: UdpSocket,
    sequence_number: u8,
    pending_acks: HashMap<SeqNum, (Instant, SerializedNetworkMessage)>,
    buffer: MsgBuffer,
    response_sender: mpsc::UnboundedSender<NetworkMessage>,
    request_receiver: mpsc::UnboundedReceiver<NetworkMessage>,
}

impl ConnectionServer {
    pub fn new() -> Result<
        (
            ConnectionServer,
            mpsc::UnboundedSender<NetworkMessage>,
            mpsc::UnboundedReceiver<NetworkMessage>,
        ),
        std::io::Error
    > {
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        socket.set_nonblocking(true);
        socket.connect("127.0.0.1:8080")?;
        let (response_sender, response_receiver) = mpsc::unbounded_channel();
        let (request_sender, request_receiver) = mpsc::unbounded_channel();

        let connection_server = ConnectionServer {
            socket,
            sequence_number: 0,
            pending_acks: HashMap::new(),
            buffer: MsgBuffer::default(),
            response_sender,
            request_receiver,
        };

        Ok((connection_server, request_sender, response_receiver))
    }
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                Some(request) = self.request_receiver.recv() => {
                    match request {
                        NetworkMessage::GetOwnServerPlayerID => {
                            todo!()
                        }
                        NetworkMessage::GetServerPlayerIDs => {
                            if let Err(e) = self.get_available_player_worlds() {
                                eprintln!("Error getting available player worlds: {}", e);
                            }
                        },
                        NetworkMessage::ClientSentWorld(sim_mem) => {
                            if let Err(e) = self.send_player_world_state(sim_mem) {
                                eprintln!("Error sending player world state: {}", e);
                            }
                        },
                        NetworkMessage::ClientSentPlayerInputs(inputs) => {
                            if let Err(e) = self.send_player_inputs(&inputs) {
                                eprintln!("Error sending player inputs: {}", e);
                            }
                        },
                        NetworkMessage::ClientConnectToOtherWorld(id) => {
                            if let Err(e) = self.connect_to_other_world(id) {
                                eprintln!("Error sending player inputs: {}", e);
                            }
                        }
                        _ => {
                            panic!("Tried to run server side NetworkMessage on client {:?}", request);
                        }
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(20)) => {
                    self.update();
                }
            }
        }
    }
    pub fn update(&mut self) {
        self.receive_messages();
        self.handle_retransmissions();
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
            crate::types::SerializedMessageType::Chunked(chunks) => {
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
                            self.pending_acks.insert(SeqNum(self.sequence_number), (
                                Instant::now(),
                                SerializedNetworkMessage { bytes: msg },
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
                return Ok(());
            }
            crate::types::SerializedMessageType::NonChunked(serialized_message) => {
                #[cfg(feature = "simulation_mode")]
                {
                    if rng_gen_range(0.0..100.0) < PACKET_LOSS_PERCENTAGE {
                        self.pending_acks.insert(SeqNum(self.sequence_number), (
                            Instant::now(),
                            serialized_message,
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
                return Ok(());
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
    fn receive_messages(&mut self) {
        loop {
            self.buffer.clear();
            match self.socket.recv(&mut self.buffer.0) {
                Ok(amt) if amt > 0 => {
                    if let Ok(request) = self.buffer.parse_on_client() {
                        if let Some(seq_num) = request.seq_num {
                            self.send_ack(SeqNum(seq_num));
                        }
                        match request.msg {
                            NetworkMessage::ServerSentWorld(data) => {}
                            NetworkMessage::ServerSentPlayerInputs(inputs) => {
                                let _ = self.response_sender.send(
                                    NetworkMessage::ServerSentPlayerInputs(inputs)
                                );
                            }
                            NetworkMessage::ServerSideAck(type_of_ack) => {
                                self.handle_ack(type_of_ack);
                            }
                            NetworkMessage::ServerSentPlayerIDs(ids) => {
                                let _ = self.response_sender.send(
                                    NetworkMessage::ServerSentPlayerIDs(ids)
                                );
                            }
                            _ => {}
                        }
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    // No more data to read
                    break;
                }
                Err(e) => {
                    eprintln!("Failed to receive: {}", e);
                    break;
                }
                _ => {
                    break;
                }
            }
        }
    }
    fn handle_ack(&mut self, type_of_ack: SeqNum) {
        LOGGER.log_received_ack(type_of_ack.0);
        self.pending_acks.remove(&type_of_ack);
        LOGGER.log_pending_acks(
            self.pending_acks
                .keys()
                .map(|k| *k)
                .collect()
        )
    }
    fn handle_retransmissions(&mut self) {
        let now = Instant::now();
        let mut to_retry = Vec::new();

        for (seq, (sent_time, request)) in self.pending_acks.iter() {
            if now.duration_since(*sent_time) > RETRY_TIMEOUT {
                to_retry.push((*seq, request.clone()));
            }
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

        // Remove messages that have exceeded max retries
        self.pending_acks.retain(|_, (sent_time, _)| {
            now.duration_since(*sent_time) < RETRY_TIMEOUT * MAX_RETRIES
        });
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
    fn send_player_inputs(&mut self, inputs: &[PlayerInput]) -> Result<(), std::io::Error> {
        if inputs.len() == 0 {
            return Ok(());
        }
        let request = NetworkMessage::ClientSentPlayerInputs(inputs.to_vec());
        self.send_unreliable(&request)
    }
}
