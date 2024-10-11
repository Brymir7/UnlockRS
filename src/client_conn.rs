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
    pub const PACKET_LOSS_PERCENTAGE: f32 = 25.0;
    pub const LATENCY_MS: Duration = Duration::from_millis(100);
}

use crate::{
    memory::PageAllocator,
    types::{
        ChunkedSerializedNetworkMessage,
        MsgBuffer,
        NetworkMessage,
        NetworkMessageType,
        PlayerInput,
        SeqNum,
        SerializedNetworkMessage,
        ServerPlayerID,
        Simulation,
    },
};

const MAX_RETRIES: u32 = 20;
const RETRY_TIMEOUT: Duration = Duration::from_millis(100);
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
                        NetworkMessage::GetPlayerInputs => {
                            todo!()
                        }
                        NetworkMessage::GetWorldState=> {
                            todo!()
                        }
                        NetworkMessage::GetServerPlayerIDs => {
                            if let Err(e) = self.get_available_player_worlds() {
                                eprintln!("Error getting available player worlds: {}", e);
                            }
                        },
                        NetworkMessage::SendWorldState(sim_mem) => {
                            if let Err(e) = self.send_player_world_state(sim_mem) {
                                eprintln!("Error sending player world state: {}", e);
                            }
                        },
                        NetworkMessage::SendPlayerInputs(inputs) => {
                            if let Err(e) = self.send_player_inputs(&inputs) {
                                eprintln!("Error sending player inputs: {}", e);
                            }
                        },
                        NetworkMessage::ConnectToOtherWorld(other_player_id, mut alloc) => {
                            match self.connect_to_other_world(other_player_id, &mut alloc) {
                                Ok(simulation) => {
                                    // Handle the new simulation
                                },
                                Err(e) => eprintln!("Error connecting to other world: {}", e),
                            }
                        },
                        _ => {
                            panic!("Tried to run server side NetworkMessage on client {:?}", request);
                        }
                    }
                },
                _ = tokio::time::sleep(Duration::from_millis(0)) => {
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
        let serialized = request.serialize(crate::types::NetworkMessageType::Unreliable);
        #[cfg(feature = "simulation_mode")]
        {
            if rng_gen_range(0.0..100.0) < PACKET_LOSS_PERCENTAGE {
                println!("Simulated packet loss for SeqNum {}", self.sequence_number);
                return Ok(());
            }
        }
        self.socket.send(&serialized.bytes)?;
        Ok(())
    }

    pub fn send_reliable(&mut self, request: &NetworkMessage) -> Result<(), std::io::Error> {
        let serialized = request.serialize(
            crate::types::NetworkMessageType::Reliable(SeqNum(self.sequence_number))
        );
        #[cfg(feature = "simulation_mode")]
        {
            if rng_gen_range(0.0..100.0) < PACKET_LOSS_PERCENTAGE {
                self.pending_acks.insert(SeqNum(self.sequence_number), (
                    Instant::now(),
                    serialized,
                ));
                self.sequence_number = self.sequence_number.wrapping_add(1);
                println!("Simulated packet loss for SeqNum {}", self.sequence_number);
                return Ok(());
            }
        }
        self.socket.send(&serialized.bytes)?;
        self.pending_acks.insert(SeqNum(self.sequence_number), (Instant::now(), serialized));
        self.sequence_number = self.sequence_number.wrapping_add(1);
        Ok(())
    }
    fn send_ack(&self, seq_num: SeqNum) {
        let ack_message = NetworkMessage::ClientSideAck(seq_num);
        let serialized_msg = ack_message.serialize(NetworkMessageType::Unreliable);
        if let Err(e) = self.socket.send(&serialized_msg.bytes) {
            eprintln!("Failed to send ACK to server: {}", e);
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
                            NetworkMessage::SendWorldState(data) => {}
                            NetworkMessage::SendPlayerInputs(inputs) => {
                                self.handle_recv_player_inputs(inputs);
                            }
                            NetworkMessage::ServerSideAck(type_of_ack) => {
                                self.handle_ack(type_of_ack);
                            }
                            NetworkMessage::SendServerPlayerIDs(ids) => {
                                let _ = self.response_sender.send(
                                    NetworkMessage::SendServerPlayerIDs(ids)
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

    fn handle_recv_player_inputs(&mut self, inputs: Vec<PlayerInput>) {
        todo!()
    }
    fn handle_ack(&mut self, type_of_ack: SeqNum) {
        println!("Received ack from server {}", type_of_ack.0);
        self.pending_acks.remove(&type_of_ack);
        println!("self.pendingacks {:?}", self.pending_acks.keys())
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
                println!("sent retranmission");
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
        let request = NetworkMessage::SendWorldState(sim_mem.clone());
        self.send_reliable(&request)
    }

    fn get_available_player_worlds(&mut self) -> Result<(), std::io::Error> {
        let request = NetworkMessage::GetServerPlayerIDs;
        self.send_reliable(&request)
    }

    fn connect_to_other_world(
        &mut self,
        other_player_id: ServerPlayerID,
        alloc: &mut PageAllocator
    ) -> Result<Simulation, std::io::Error> {
        todo!();
    }

    fn send_player_inputs(&mut self, inputs: &[PlayerInput]) -> Result<(), std::io::Error> {
        if inputs.len() == 0 {
            return Ok(());
        }
        let request = NetworkMessage::SendPlayerInputs(inputs.to_vec());
        self.send_reliable(&request)
    }

}
