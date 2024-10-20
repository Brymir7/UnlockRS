use std::{ fmt::Display, fs::OpenOptions, time::Instant };

use crate::types::{
    BufferedNetworkedPlayerInputs,
    ChunkOfMessage,
    ChunkedMessageCollector,
    ChunkedSerializedNetworkMessage,
    DeserializedMessage,
    DeserializedMessageType,
    LogConfig,
    Logger,
    MessageHeader,
    MsgBuffer,
    NetworkLogger,
    NetworkMessage,
    NetworkMessageType,
    NetworkedPlayerInput,
    PacketParser,
    PlayerID,
    PlayerInput,
    SeqNum,
    SeqNumGenerator,
    SerializedMessageType,
    SerializedNetworkMessage,
    ServerPlayerID,
    AMT_OF_CHUNKS_BYTE_POS,
    AMT_RANDOM_BYTES,
    BASE_CHUNK_SEQ_NUM_BYTE_POS,
    DATA_BIT_START_POS,
    DISCRIMINANT_BIT_START_POS,
    MAX_UDP_PAYLOAD_DATA_LENGTH,
    MAX_UDP_PAYLOAD_LEN,
    PLAYER_MOVE_LEFT_BYTE_POS,
    PLAYER_MOVE_RIGHT_BYTE_POS,
    PLAYER_SHOOT_BYTE_POS,
    RELIABLE_FLAG_BYTE_POS,
    SEQ_NUM_BYTE_POS,
    VECTOR_LEN_BYTE_POS,
};
impl PacketParser {
    pub fn parse_header(bytes: &[u8]) -> Result<MessageHeader, &'static str> {
        let reliable = bytes[RELIABLE_FLAG_BYTE_POS] > 0;
        let seq_num = if reliable {
            Some(SeqNum(u16::from_le_bytes([bytes[SEQ_NUM_BYTE_POS], bytes[SEQ_NUM_BYTE_POS + 1]])))
        } else {
            None
        };
        let amt_of_chunks = u16::from_le_bytes([
            bytes[AMT_OF_CHUNKS_BYTE_POS],
            bytes[AMT_OF_CHUNKS_BYTE_POS + 1],
        ]);
        let base_chunk_seq_num = u16::from_le_bytes([
            bytes[BASE_CHUNK_SEQ_NUM_BYTE_POS],
            bytes[BASE_CHUNK_SEQ_NUM_BYTE_POS + 1],
        ]);
        let is_chunked = amt_of_chunks > 0;
        let discriminator = bytes[DISCRIMINANT_BIT_START_POS];
        let message = NetworkMessage::try_from(discriminator)?;

        Ok(MessageHeader {
            reliable,
            seq_num,
            amt_of_chunks,
            base_chunk_seq_num,
            is_chunked,
            message,
        })
    }
    fn parse_data(
        header: &MessageHeader,
        data: &[u8]
    ) -> Result<DeserializedMessage, &'static str> {
        debug_assert!(data.len() % MAX_UDP_PAYLOAD_DATA_LENGTH == 0, "data.len {}", data.len()); // either its 1 packet or its multiple packets of this size
        // HEADER IS REMOVED from data; ONLY DATA HERE
        let parsed_message = match header.message {
            | NetworkMessage::GetServerPlayerIDs
            | NetworkMessage::GetOwnServerPlayerID
            | NetworkMessage::ServerRequestHostForWorldData => header.message.clone(),

            NetworkMessage::ClientSentWorld(_) => NetworkMessage::ClientSentWorld(data.to_vec()),

            | NetworkMessage::ClientSentPlayerInputs(_)
            | NetworkMessage::ServerSentPlayerInputs(_) => {
                let mut buffered_inputs = BufferedNetworkedPlayerInputs::default();
                let mut offset = 1; // Start after the first byte, which is the length of the Vec
                let input_count = data[0] as usize;
                for _ in 0..input_count {
                    let frame = u32::from_le_bytes(data[offset..offset + 4].try_into().unwrap());
                    offset += 4;
                    let player_inputs = parse_player_inputs(data[offset]);
                    offset += 1;
                    buffered_inputs.buffered_inputs.push(NetworkedPlayerInput {
                        inputs: player_inputs,
                        frame,
                    });
                }
                match header.message {
                    NetworkMessage::ClientSentPlayerInputs(_) => {
                        NetworkMessage::ClientSentPlayerInputs(buffered_inputs)
                    }
                    NetworkMessage::ServerSentPlayerInputs(_) => {
                        NetworkMessage::ServerSentPlayerInputs(buffered_inputs)
                    }
                    _ => { panic!() }
                }
            }

            NetworkMessage::ClientConnectToOtherWorld(_) => {
                NetworkMessage::ClientConnectToOtherWorld(ServerPlayerID(data[0]))
            }
            NetworkMessage::ServerSideAck(_) | NetworkMessage::ClientSideAck(_) => {
                if data.len() < std::mem::size_of::<SeqNum>() {
                    return Err("Insufficient data for Ack message");
                }
                let seq_num = SeqNum(u16::from_le_bytes([data[0], data[1]])); // Assuming SeqNum is a single byte
                match header.message {
                    NetworkMessage::ServerSideAck(_) => NetworkMessage::ServerSideAck(seq_num),
                    NetworkMessage::ClientSideAck(_) => NetworkMessage::ClientSideAck(seq_num),
                    _ => unreachable!(),
                }
            }

            NetworkMessage::ServerSentPlayerIDs(_) => {
                let amt = data[0] as usize;
                println!("server sent player ids amt {}", amt);
                println!("{:?}", data);
                debug_assert!(amt + 1 < data.len());
                NetworkMessage::ServerSentPlayerIDs(data[1..amt + 1].to_vec())
            }

            NetworkMessage::ServerSentWorld(_) => NetworkMessage::ServerSentWorld(data.to_vec()),
        };

        if header.reliable {
            Ok(
                DeserializedMessage::from_reliable_msg(
                    parsed_message,
                    header.seq_num.map(|s| s.0)
                )
            )
        } else {
            Ok(DeserializedMessage::from_unreliable_msg(parsed_message))
        }
    }
}
impl MsgBuffer {
    pub fn default() -> MsgBuffer {
        MsgBuffer([0; MAX_UDP_PAYLOAD_LEN])
    }
    pub fn clear(&mut self) {
        self.0 = [0; MAX_UDP_PAYLOAD_LEN];
    }

    pub fn parse_on_server(&self) -> Result<DeserializedMessageType, &'static str> {
        let bytes = &self.0;
        if bytes.is_empty() {
            return Err("Empty buffer");
        }
        let header = PacketParser::parse_header(bytes)?;

        // Debug assert to ensure only client-sent events are received on the server
        debug_assert!(
            matches!(
                header.message,
                NetworkMessage::GetServerPlayerIDs |
                    NetworkMessage::GetOwnServerPlayerID |
                    NetworkMessage::ClientSentWorld(_) |
                    NetworkMessage::ClientSentPlayerInputs(_) |
                    NetworkMessage::ClientSideAck(_) |
                    NetworkMessage::ClientConnectToOtherWorld(_)
            ),
            "Server received an invalid message type: {:?}",
            header.message
        );

        if header.is_chunked {
            return Ok(
                DeserializedMessageType::ChunkOfMessage(ChunkOfMessage {
                    seq_num: header.seq_num.unwrap().0,
                    base_seq_num: header.base_chunk_seq_num,
                    amt_of_chunks: header.amt_of_chunks,
                    data_bytes: *bytes,
                })
            );
        }
        let parsed_data = PacketParser::parse_data(&header, &bytes[DATA_BIT_START_POS..].to_vec())?;

        Ok(DeserializedMessageType::NonChunked(parsed_data))
    }

    pub fn parse_on_client(&self) -> Result<DeserializedMessageType, &'static str> {
        let bytes = &self.0;

        if bytes.is_empty() {
            return Err("Empty buffer");
        }
        let header = PacketParser::parse_header(bytes)?;
        // Debug assert to ensure only server-sent events are received on the client
        debug_assert!(
            matches!(
                header.message,
                NetworkMessage::ServerSideAck(_) |
                    NetworkMessage::ServerSentPlayerIDs(_) |
                    NetworkMessage::ServerSentPlayerInputs(_) |
                    NetworkMessage::ServerSentWorld(_) |
                    NetworkMessage::ServerRequestHostForWorldData
            ),
            "Client received an invalid message type: {:?}",
            header.message
        );
        if header.is_chunked {
            return Ok(
                DeserializedMessageType::ChunkOfMessage(ChunkOfMessage {
                    seq_num: header.seq_num.unwrap().0,
                    base_seq_num: header.base_chunk_seq_num,
                    amt_of_chunks: header.amt_of_chunks,
                    data_bytes: *bytes,
                })
            );
        }
        let parsed_data = PacketParser::parse_data(&header, &bytes[DATA_BIT_START_POS..].to_vec())?;
        Ok(DeserializedMessageType::NonChunked(parsed_data))
    }
}
fn parse_player_inputs(byte: u8) -> Vec<PlayerInput> {
    let mut res = Vec::new();
    let player_moves_left = (byte >> PLAYER_MOVE_LEFT_BYTE_POS) & 1;
    let player_moves_right: u8 = (byte >> PLAYER_MOVE_RIGHT_BYTE_POS) & 1;
    let player_shoots: u8 = (byte >> PLAYER_SHOOT_BYTE_POS) & 1;
    if player_moves_left > 0 {
        res.push(PlayerInput::Left);
    }
    if player_moves_right > 0 {
        res.push(PlayerInput::Right);
    }
    if player_shoots > 0 {
        res.push(PlayerInput::Shoot);
    }
    return res;
}
impl DeserializedMessage {
    fn from_reliable_msg(msg: NetworkMessage, seq_num: Option<u16>) -> Self {
        DeserializedMessage {
            reliable: true,
            seq_num,
            msg: msg,
        }
    }
    fn from_unreliable_msg(msg: NetworkMessage) -> Self {
        DeserializedMessage {
            reliable: false,
            seq_num: None,
            msg: msg,
        }
    }
}
use rand::Rng;
impl NetworkMessage {
    pub fn chunk_message(
        &self,
        discriminator_byte: u8,
        data: &Vec<u8>,
        msg_type: NetworkMessageType
    ) -> SerializedMessageType {
        let amt_of_chunks =
            (data.len() + MAX_UDP_PAYLOAD_DATA_LENGTH - 1) / MAX_UDP_PAYLOAD_DATA_LENGTH;
        debug_assert!(amt_of_chunks < (u8::MAX as usize), "{}", amt_of_chunks);
        let mut byte_chunks: Vec<Vec<u8>> = Vec::new();
        let mut rng = rand::thread_rng();
        let random_bytes: Vec<u8> = (0..AMT_RANDOM_BYTES).map(|_| rng.gen()).collect(); // First few random bytes (3 bytes in this example)
        for i in 0..amt_of_chunks {
            let mut msg_bytes = Vec::new();
            match msg_type {
                NetworkMessageType::ResendUntilAck(seq_num) => {
                    msg_bytes.extend(random_bytes.clone());
                    msg_bytes.push(1); // true
                    msg_bytes.extend_from_slice(&seq_num.0.wrapping_add(i as u16).to_le_bytes());
                    msg_bytes.extend_from_slice(&seq_num.0.to_le_bytes());
                    msg_bytes.extend_from_slice(&(amt_of_chunks as u16).to_le_bytes());
                    msg_bytes.push(discriminator_byte);

                    debug_assert!(msg_bytes[RELIABLE_FLAG_BYTE_POS] == 1);
                    debug_assert!(
                        u16::from_le_bytes([
                            msg_bytes[SEQ_NUM_BYTE_POS],
                            msg_bytes[SEQ_NUM_BYTE_POS + 1],
                        ]) == seq_num.0.wrapping_add(i as u16)
                    );
                    debug_assert!(
                        u16::from_le_bytes([
                            msg_bytes[BASE_CHUNK_SEQ_NUM_BYTE_POS],
                            msg_bytes[BASE_CHUNK_SEQ_NUM_BYTE_POS + 1],
                        ]) == seq_num.0
                    );
                    debug_assert!(
                        u16::from_le_bytes([
                            msg_bytes[AMT_OF_CHUNKS_BYTE_POS],
                            msg_bytes[AMT_OF_CHUNKS_BYTE_POS + 1],
                        ]) == (amt_of_chunks as u16)
                    );
                    debug_assert!(msg_bytes[DISCRIMINANT_BIT_START_POS] == discriminator_byte);
                }
                NetworkMessageType::SendOnce | NetworkMessageType::SendOnceButReceiveAck(_) => {
                    panic!("Cannot send chunked message unreliable");
                }
            }
            msg_bytes.extend(
                &data
                    [
                        i * MAX_UDP_PAYLOAD_DATA_LENGTH..(
                            (i + 1) *
                            MAX_UDP_PAYLOAD_DATA_LENGTH
                        ).min(data.len())
                    ]
            );
            byte_chunks.push(msg_bytes);
        }
        return SerializedMessageType::from_chunked_msg(byte_chunks);
    }
    pub fn push_non_chunked(bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&(0 as u16).to_le_bytes());
        bytes.extend_from_slice(&(0 as u16).to_le_bytes());
        debug_assert!(
            u16::from_le_bytes([
                bytes[BASE_CHUNK_SEQ_NUM_BYTE_POS],
                bytes[BASE_CHUNK_SEQ_NUM_BYTE_POS + 1],
            ]) == 0
        );
        debug_assert!(
            u16::from_le_bytes([
                bytes[AMT_OF_CHUNKS_BYTE_POS],
                bytes[AMT_OF_CHUNKS_BYTE_POS + 1],
            ]) == 0
        );
    }
    pub fn serialize(&self, msg_type: NetworkMessageType) -> SerializedMessageType {
        let msg = self.may_overflow_udp_packet_serialize(msg_type);
        match &msg {
            SerializedMessageType::Chunked(_) => {}
            SerializedMessageType::NonChunked(msg) =>
                debug_assert!(msg.bytes.len() < MAX_UDP_PAYLOAD_LEN),
        }
        return msg;
    }
    pub fn may_overflow_udp_packet_serialize(
        &self,
        msg_type: NetworkMessageType
    ) -> SerializedMessageType {
        let mut rng = rand::thread_rng();
        let mut bytes: Vec<u8> = Vec::new();
        let random_bytes: Vec<u8> = (0..AMT_RANDOM_BYTES).map(|_| rng.gen()).collect(); // First few random bytes (3 bytes in this example)
        bytes.extend(random_bytes);
        match msg_type {
            | NetworkMessageType::ResendUntilAck(seq_num)
            | NetworkMessageType::SendOnceButReceiveAck(seq_num) => {
                bytes.push(1); // true
                bytes.extend_from_slice(&seq_num.0.to_le_bytes());
                debug_assert!(bytes[RELIABLE_FLAG_BYTE_POS] == 1);
                debug_assert!(
                    u16::from_le_bytes([bytes[SEQ_NUM_BYTE_POS], bytes[SEQ_NUM_BYTE_POS + 1]]) ==
                        seq_num.0
                );
            }
            NetworkMessageType::SendOnce => {
                bytes.push(0);
                // seq num is u16
                bytes.push(0);
                bytes.push(0);
                //

                debug_assert!(bytes[RELIABLE_FLAG_BYTE_POS] == 0);
                debug_assert!(
                    u16::from_le_bytes([bytes[SEQ_NUM_BYTE_POS], bytes[SEQ_NUM_BYTE_POS + 1]]) == 0
                );
            }
        }

        match *self {
            Self::ClientSentWorld(ref sim) | Self::ServerSentWorld(ref sim) => {
                let discriminator: u8 = match *self {
                    Self::ClientSentWorld(_) => {
                        NetworkMessage::ClientSentWorld(Vec::new()).into()
                    }
                    Self::ServerSentWorld(_) => {
                        NetworkMessage::ServerSentWorld(Vec::new()).into()
                    }
                    _ => { panic!() }
                };
                if sim.len() > MAX_UDP_PAYLOAD_DATA_LENGTH {
                    println!("chunking message");
                    return self.chunk_message(discriminator, &sim, msg_type);
                } else {
                    Self::push_non_chunked(&mut bytes);
                    bytes.push(discriminator);
                    bytes.extend(sim); // append actual Vec<u8> data
                    return SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                        bytes,
                    });
                }
            }
            Self::ClientSentPlayerInputs(ref inp) | Self::ServerSentPlayerInputs(ref inp) => {
                Self::push_non_chunked(&mut bytes);
                let message = match *self {
                    Self::ClientSentPlayerInputs(_) => {
                        NetworkMessage::ClientSentPlayerInputs(
                            BufferedNetworkedPlayerInputs::default()
                        )
                    }
                    Self::ServerSentPlayerInputs(_) => {
                        NetworkMessage::ServerSentPlayerInputs(
                            BufferedNetworkedPlayerInputs::default()
                        )
                    }
                    _ => { panic!() }
                };
                bytes.push(message.into());
                bytes.push(inp.buffered_inputs.len() as u8);
                for input in &inp.buffered_inputs {
                    let packed_inputs = Self::pack_player_inputs(&input.inputs);
                    bytes.extend_from_slice(&input.frame.to_le_bytes());
                    bytes.push(packed_inputs);
                }
                debug_assert!(bytes.len() <= MAX_UDP_PAYLOAD_LEN, "length {}", bytes.len());
                SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                    bytes,
                })
            }

            Self::ServerSideAck(ref seq_num) => {
                Self::push_non_chunked(&mut bytes);
                bytes.push(NetworkMessage::ServerSideAck(SeqNum(0)).into());
                bytes.extend_from_slice(&seq_num.0.to_le_bytes());
                SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                    bytes,
                })
            }
            Self::ClientSideAck(ref seq_num) => {
                Self::push_non_chunked(&mut bytes);
                bytes.push(NetworkMessage::ClientSideAck(SeqNum(0)).into());
                bytes.extend_from_slice(&seq_num.0.to_le_bytes());
                SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                    bytes,
                })
            }
            Self::ServerSentPlayerIDs(ref ids) => {
                Self::push_non_chunked(&mut bytes);
                bytes.push(NetworkMessage::ServerSentPlayerIDs(Vec::new()).into());
                debug_assert!(ids.len() <= (u8::MAX as usize));
                bytes.push(ids.len() as u8);
                bytes.extend(ids);
                println!(
                    "length of server send ids {} vs bytes [VECTOR_LEN_BYTE_POS] {}",
                    ids.len() as u8,
                    bytes[VECTOR_LEN_BYTE_POS]
                );
                debug_assert!(bytes[VECTOR_LEN_BYTE_POS] == (ids.len() as u8));
                SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                    bytes,
                })
            }
            Self::ClientConnectToOtherWorld(ref id) => {
                Self::push_non_chunked(&mut bytes);
                bytes.push(NetworkMessage::ClientConnectToOtherWorld(ServerPlayerID(0)).into());
                bytes.push(id.0);
                SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                    bytes,
                })
            }
            _ => {
                Self::push_non_chunked(&mut bytes);
                bytes.push(self.into());
                SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                    bytes,
                })
            }
        }
    }

    fn pack_player_inputs(inputs: &Vec<PlayerInput>) -> u8 {
        let mut res: u8 = 0;
        for input in inputs {
            match *input {
                PlayerInput::Left => {
                    res = res | (1 << PLAYER_MOVE_LEFT_BYTE_POS);
                }
                PlayerInput::Right => {
                    res = res | (1 << PLAYER_MOVE_RIGHT_BYTE_POS);
                }
                PlayerInput::Shoot => {
                    res = res | (1 << PLAYER_SHOOT_BYTE_POS);
                }
            }
        }
        return res;
    }
}
impl From<NetworkMessage> for u8 {
    fn from(request: NetworkMessage) -> u8 {
        match request {
            NetworkMessage::GetServerPlayerIDs => 0,
            NetworkMessage::GetOwnServerPlayerID => 1,
            NetworkMessage::ClientSentWorld(_) => 2,
            NetworkMessage::ClientSentPlayerInputs(_) => 3,
            NetworkMessage::ServerSideAck(_) => 4,
            NetworkMessage::ClientSideAck(_) => 5,
            NetworkMessage::ServerSentPlayerIDs(_) => 6,
            NetworkMessage::ServerSentPlayerInputs(_) => 7,
            NetworkMessage::ServerSentWorld(_) => 8,
            NetworkMessage::ClientConnectToOtherWorld(_) => 9,
            NetworkMessage::ServerRequestHostForWorldData => 10,
        }
    }
}
impl From<&NetworkMessage> for u8 {
    fn from(request: &NetworkMessage) -> u8 {
        match request {
            NetworkMessage::GetServerPlayerIDs => 0,
            NetworkMessage::GetOwnServerPlayerID => 1,
            NetworkMessage::ClientSentWorld(_) => 2,
            NetworkMessage::ClientSentPlayerInputs(_) => 3,
            NetworkMessage::ServerSideAck(_) => 4,
            NetworkMessage::ClientSideAck(_) => 5,
            NetworkMessage::ServerSentPlayerIDs(_) => 6,
            NetworkMessage::ServerSentPlayerInputs(_) => 7,
            NetworkMessage::ServerSentWorld(_) => 8,
            NetworkMessage::ClientConnectToOtherWorld(_) => 9,
            NetworkMessage::ServerRequestHostForWorldData => 10,
        }
    }
}
// Implementing TryFrom to convert u8 back into NetworkMessage
impl TryFrom<u8> for NetworkMessage {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(NetworkMessage::GetServerPlayerIDs),
            1 => Ok(NetworkMessage::GetOwnServerPlayerID),

            2 => Ok(NetworkMessage::ClientSentWorld(Vec::new())),
            3 =>
                Ok(
                    NetworkMessage::ClientSentPlayerInputs(BufferedNetworkedPlayerInputs::default())
                ),

            4 => Ok(NetworkMessage::ServerSideAck(SeqNum(0))),
            5 => Ok(NetworkMessage::ClientSideAck(SeqNum(0))),

            6 => Ok(NetworkMessage::ServerSentPlayerIDs(Vec::new())),
            7 =>
                Ok(
                    NetworkMessage::ServerSentPlayerInputs(BufferedNetworkedPlayerInputs::default())
                ),
            8 => Ok(NetworkMessage::ServerSentWorld(Vec::new())),
            9 => Ok(NetworkMessage::ClientConnectToOtherWorld(ServerPlayerID(0))),
            10 => Ok(NetworkMessage::ServerRequestHostForWorldData),
            _ => {
                println!("Invalid value : {}", value);
                Err("Invalid network msg u8 type ^^")
            }
        }
    }
}

impl SerializedMessageType {
    fn from_serialized_msg(msg: SerializedNetworkMessage) -> Self {
        return SerializedMessageType::NonChunked(msg);
    }
    fn from_chunked_msg(msgs: Vec<Vec<u8>>) -> Self {
        return SerializedMessageType::Chunked(ChunkedSerializedNetworkMessage {
            bytes: msgs,
        });
    }
}

impl ChunkedMessageCollector {
    pub fn default() -> Self {
        let mut msgs = Vec::with_capacity(u16::MAX as usize); // TODO THIS is inefficient
        for _ in 0..u16::MAX {
            msgs.push(Vec::new());
        }
        return ChunkedMessageCollector {
            msgs: msgs,
        };
    }
    pub fn collect(&mut self, chunk: ChunkOfMessage) {
        self.msgs[chunk.base_seq_num as usize].push(chunk);
    }
    pub fn try_combine(&mut self) -> Option<DeserializedMessage> {
        for msg in &mut self.msgs {
            msg.sort_by_key(|chunk| chunk.seq_num); // 0 is after 255 due tu rounding but not respected here TODO

            if let Some(last_msg) = msg.last() {
                if
                    last_msg.seq_num ==
                        last_msg.base_seq_num.wrapping_add(last_msg.amt_of_chunks - 1) && // first packet will have base_Seq_num so last packet wioll be amt_ofchunks-1 away
                    (last_msg.amt_of_chunks as usize) == msg.len()
                {
                    let total_data_bytes: Vec<u8> = msg
                        .iter()
                        .flat_map(|chunk| chunk.data_bytes[DATA_BIT_START_POS..].to_vec())
                        .collect();
                    debug_assert!(msg[0].seq_num == msg[0].base_seq_num);
                    debug_assert!(msg[0].seq_num <= last_msg.seq_num);
                    let header = PacketParser::parse_header(&msg[0].data_bytes);
                    match header {
                        Ok(header) => {
                            let deserialized_message = PacketParser::parse_data(
                                &header,
                                &total_data_bytes
                            );
                            match deserialized_message {
                                Ok(deserialized_message) => {
                                    msg.clear();
                                    return Some(deserialized_message);
                                }
                                Err(e) => eprintln!("Failed to parse data of chunk: {}", e),
                            }
                        }
                        Err(e) => {
                            eprintln!("Error when parsing header from chunk: {}", e);
                        }
                    }
                }
            }
        }
        return None;
    }
}

impl NetworkLogger {
    pub fn log_simulated_packet_loss(&self, sequence_number: u16) {
        if self.log {
            println!("Simulated packet loss for SeqNum {}", sequence_number);
        }
    }
    pub fn log_received_ack(&self, ack_num: u16) {
        if self.log {
            println!("Received ack from server: {}", ack_num);
        }
    }
    pub fn log_pending_acks(&self, pending: Vec<SeqNum>) {
        if self.log {
            println!("Currently pending acks: {:?}", pending)
        }
    }
    pub fn log_sent_retransmission(&self, seq_num: u16) {
        if self.log {
            println!("Sent retransmission for SeqNum: {}", seq_num);
        }
    }
    pub fn log_sent_packet(&self, seq_num: u16) {
        if self.log {
            println!("Sent packet {}", seq_num);
        }
    }
}

impl NetworkedPlayerInput {
    pub fn new(inputs: Vec<PlayerInput>, frame: u32) -> Self {
        NetworkedPlayerInput {
            inputs,
            frame,
        }
    }
    pub fn placeholder() -> Self {
        NetworkedPlayerInput {
            inputs: Vec::new(),
            frame: 0,
        }
    }
}

impl PlayerID {
    pub fn from_usize(u: usize) -> Option<PlayerID> {
        match u {
            0 => Some(PlayerID::Player1),
            1 => Some(PlayerID::Player2),
            _ => None,
        }
    }
}
use std::io::{ self, Write };
impl BufferedNetworkedPlayerInputs {
    pub fn default() -> Self {
        BufferedNetworkedPlayerInputs {
            buffered_inputs: Vec::new(),
        }
    }
    pub fn bulk_insert_player_input(&mut self, other: BufferedNetworkedPlayerInputs) {
        for networked_input in other.buffered_inputs {
            if
                let None = self.buffered_inputs
                    .iter_mut()
                    .find(|i| i.frame == networked_input.frame)
            {
                // Insert new NetworkedPlayerInput if frame doesn't exist
                self.buffered_inputs.push(networked_input);
            }
        }
        debug_assert!(
            self.buffered_inputs.iter().all(|input| {
                self.buffered_inputs
                    .iter()
                    .filter(|other_inp| **other_inp == *input)
                    .count() == 1
            })
        );
    }
    pub fn insert_player_input(&mut self, networked_input: NetworkedPlayerInput) {
        if let None = self.buffered_inputs.iter_mut().find(|i| i.frame == networked_input.frame) {
            // Insert new NetworkedPlayerInput if frame doesn't exist
            self.buffered_inputs.push(networked_input);
        }

        debug_assert!(
            self.buffered_inputs.iter().all(|input| {
                self.buffered_inputs
                    .iter()
                    .filter(|other_inp| **other_inp == *input)
                    .count() == 1
            })
        );
    }

    pub fn discard_acknowledged_frames(&mut self, frame: u32) {
        // let discarded_frames: Vec<u32> = self.buffered_inputs
        //     .iter()
        //     .filter(|input| input.frame < frame)
        //     .map(|input| input.frame)
        //     .collect();

        // // Log discarded frames to a file
        // if !discarded_frames.is_empty() {
        //     let file_path = "discarded_frames.log";
        //     let mut file = OpenOptions::new()
        //         .create(true)
        //         .append(true)
        //         .open(file_path)
        //         .expect("Failed to open log file");

        //     for frame in discarded_frames {
        //         writeln!(file, "Discarded frame: {}", frame).expect("Failed to write to log file");
        //     }
        // }

        self.buffered_inputs.retain(|input| input.frame > frame);
        debug_assert!(
            self.buffered_inputs.iter().all(|input| input.frame > frame),
            "There are frames that are less than the acknowledged frame"
        );
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            connection: false,
            world_state: false,
            player_input: false,
            message_handling: false,
            ack: false,
            error: false,
            debug: false,
        }
    }
}

impl Logger {
    pub fn new(config: LogConfig) -> Self {
        Self { config, last_log_time: None }
    }

    pub fn connection<T: Display>(&self, message: T) {
        if self.config.connection {
            println!("[CONNECTION] {}", message);
        }
    }

    pub fn world_state<T: Display>(&self, message: T) {
        if self.config.world_state {
            println!("[WORLD_STATE] {}", message);
        }
    }

    pub fn player_input<T: Display>(&self, message: T) {
        if self.config.player_input {
            println!("[PLAYER_INPUT] {}", message);
        }
    }

    pub fn message<T: Display>(&self, message: T) {
        if self.config.message_handling {
            println!("[MESSAGE] {}", message);
        }
    }

    pub fn ack<T: Display>(&self, message: T) {
        if self.config.ack {
            println!("[ACK] {}", message);
        }
    }

    pub fn error<T: Display>(&self, message: T) {
        if self.config.error {
            eprintln!("[ERROR] {}", message);
        }
    }

    pub fn debug<T: Display>(&self, message: T) {
        if self.config.debug {
            println!("[DEBUG] {}", message);
        }
    }
    pub fn debug_log_time<T: Display>(&mut self, message: T) {
        if self.config.debug {
            let now = Instant::now();

            if let Some(last_time) = self.last_log_time {
                let delta = now.duration_since(last_time);
                println!("[DEBUG] {} | Time: {:?} | Delta: {:?}", message, now, delta);
            } else {
                println!("[DEBUG] {}", message);
            }

            // Update the last log time
            self.last_log_time = Some(now);
        }
    }
}

impl SeqNumGenerator {
    pub fn get_seq_num(&mut self) -> SeqNum {
        let num = self.seq_num;
        self.seq_num = SeqNum(self.seq_num.0.wrapping_add(1));
        return num;
    }
}
