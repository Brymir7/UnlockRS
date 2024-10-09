use std::{ collections::HashMap, net::UdpSocket, time::{ Duration, Instant } };

use crate::{
    memory::PageAllocator,
    types::{
        ChunkedSerializedNetworkMessage,
        MsgBuffer,
        NetworkMessage,
        PlayerInput,
        SeqNum,
        SerializedNetworkMessage,
        ServerPlayerID,
        Simulation,
    },
};

const MAX_RETRIES: u32 = 5;
const RETRY_TIMEOUT: Duration = Duration::from_millis(100);

pub struct ConnectionServer {
    socket: UdpSocket,
    sequence_number: u8,
    pending_acks: HashMap<SeqNum, (Instant, SerializedNetworkMessage)>,
    buffer: MsgBuffer,
}

impl ConnectionServer {
    pub fn new() -> Result<Self, std::io::Error> {
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        socket.connect("127.0.0.1:8080")?;
        socket.set_nonblocking(true)?;

        Ok(ConnectionServer {
            socket,
            sequence_number: 0,
            pending_acks: HashMap::new(),
            buffer: MsgBuffer::default(),
        })
    }

    pub fn update(&mut self) {
        self.receive_messages();
        self.handle_retransmissions();
    }

    pub fn send_unreliable(&self, request: &NetworkMessage) -> Result<(), std::io::Error> {
        let serialized = request.serialize(crate::types::NetworkMessageType::Unreliable);
        self.socket.send(&serialized.bytes)?;
        Ok(())
    }

    pub fn send_reliable(&mut self, request: &NetworkMessage) -> Result<(), std::io::Error> {
        let serialized = request.serialize(
            crate::types::NetworkMessageType::Reliable(SeqNum(self.sequence_number))
        );
        self.socket.send(&serialized.bytes)?;
        self.pending_acks.insert(SeqNum(self.sequence_number), (Instant::now(), serialized));
        self.sequence_number = self.sequence_number.wrapping_add(1);
        Ok(())
    }

    fn receive_messages(&mut self) {
        loop {
            self.buffer.clear();
            match self.socket.recv(&mut self.buffer.0) {
                Ok(amt) if amt > 0 => {
                    if let Ok(request) = self.buffer.parse_on_client() {
                        match request {
                            NetworkMessage::SendWorldState(data) => {}
                            NetworkMessage::SendPlayerInputs(inputs) => {
                                self.handle_recv_player_inputs(inputs);
                            }
                            NetworkMessage::ServerSideAck(type_of_ack) => {
                                self.handle_ack(type_of_ack);
                            }
                            NetworkMessage::SendServerPlayerIDs(ids) => {
                                println!("received server player ids {:?}", ids);
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
        println!("Received ack from server");
        self.pending_acks.remove(&type_of_ack);
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

    pub fn send_player_world_state(
        &mut self,
        sim_mem: &PageAllocator
    ) -> Result<(), std::io::Error> {
        let request = NetworkMessage::SendWorldState(sim_mem.get_copy_of_state());
        self.send_reliable(&request)
    }

    pub fn get_available_player_worlds(&mut self) {
        let request = NetworkMessage::GetServerPlayerIDs;
        self.send_reliable(&request);
    }

    pub fn connect_to_other_world(
        &mut self,
        other_player_id: ServerPlayerID,
        alloc: &mut PageAllocator
    ) -> Result<Simulation, std::io::Error> {
        todo!();
    }

    pub fn send_player_inputs(&mut self, inputs: &[PlayerInput]) -> Result<(), std::io::Error> {
        if inputs.len() == 0 {
            return Ok(());
        }
        let request = NetworkMessage::SendPlayerInputs(inputs.to_vec());
        self.send_reliable(&request)
    }
}
