use crate::types::{
    ChunkOfMessage,
    ChunkedMessageCollector,
    ChunkedSerializedNetworkMessage,
    DeserializedMessage,
    DeserializedMessageType,
    MessageHeader,
    MsgBuffer,
    NetworkLogger,
    NetworkMessage,
    NetworkMessageType,
    PacketParser,
    Player,
    PlayerInput,
    SeqNum,
    SerializedMessageType,
    SerializedNetworkMessage,
    ServerPlayerID,
    AMT_OF_CHUNKS_BYTE_POS,
    AMT_RANDOM_BYTES,
    BASE_CHUNK_SEQ_NUM_BYTE_POS,
    DATA_BIT_START_POS,
    DISCRIMINANT_BIT_START_POS,
    MAX_UDP_PAYLOAD_LEN,
    PAYLOAD_DATA_LENGTH,
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
        let seq_num = if reliable { Some(SeqNum(bytes[SEQ_NUM_BYTE_POS])) } else { None };
        let amt_of_chunks = bytes[AMT_OF_CHUNKS_BYTE_POS];
        let base_chunk_seq_num = bytes[BASE_CHUNK_SEQ_NUM_BYTE_POS];
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
        let parsed_message = match header.message {
            NetworkMessage::GetServerPlayerIDs | NetworkMessage::GetOwnServerPlayerID =>
                header.message.clone(),

            NetworkMessage::ClientSentWorld(_) => NetworkMessage::ClientSentWorld(data.to_vec()),

            NetworkMessage::ClientSentPlayerInputs(_) => {
                let player_inputs = parse_player_inputs(&data);
                NetworkMessage::ClientSentPlayerInputs(player_inputs)
            }
            NetworkMessage::ClientConnectToOtherWorld(_) => {
                NetworkMessage::ClientConnectToOtherWorld(ServerPlayerID(data[0]))
            }
            NetworkMessage::ServerSideAck(_) | NetworkMessage::ClientSideAck(_) => {
                if data.len() < std::mem::size_of::<SeqNum>() {
                    return Err("Insufficient data for Ack message");
                }
                let seq_num = SeqNum(data[0]); // Assuming SeqNum is a single byte
                match header.message {
                    NetworkMessage::ServerSideAck(_) => NetworkMessage::ServerSideAck(seq_num),
                    NetworkMessage::ClientSideAck(_) => NetworkMessage::ClientSideAck(seq_num),
                    _ => unreachable!(),
                }
            }

            NetworkMessage::ServerSentPlayerIDs(_) =>
                NetworkMessage::ServerSentPlayerIDs(data.to_vec()),

            NetworkMessage::ServerSentPlayerInputs(_) => {
                let player_inputs = parse_player_inputs(&data);
                NetworkMessage::ServerSentPlayerInputs(player_inputs)
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
                    NetworkMessage::ClientSideAck(_)
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

    pub fn parse_on_client(&self) -> Result<DeserializedMessage, &'static str> {
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
                    NetworkMessage::ServerSentWorld(_)
            ),
            "Client received an invalid message type: {:?}",
            header.message
        );

        let parsed_data = PacketParser::parse_data(&header, &bytes[DATA_BIT_START_POS..].to_vec())?;
        Ok(parsed_data)
    }
}
fn parse_player_inputs(bytes: &[u8]) -> Vec<PlayerInput> {
    let mut res = Vec::new();
    let byte = bytes[0];
    let player_moves_left = (byte >> PLAYER_MOVE_LEFT_BYTE_POS) & 1;
    let player_moves_right: u8 = (byte >> PLAYER_MOVE_RIGHT_BYTE_POS) & 1;
    let player_shoots: u8 = (byte >> PLAYER_SHOOT_BYTE_POS) & 1;
    if player_moves_left > 0 {
        res.push(PlayerInput::Left);
    }
    if player_moves_right > 0 {
        res.push(PlayerInput::Right);
    }
    println!("player_shoots {}", player_shoots);
    if player_shoots > 0 {
        res.push(PlayerInput::Shoot);
    }
    return res;
}
impl DeserializedMessage {
    fn from_reliable_msg(msg: NetworkMessage, seq_num: Option<u8>) -> Self {
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
use rand::{ seq, Rng };
impl NetworkMessage {
    pub fn chunk_message(
        &self,
        discriminator_byte: u8,
        data: &Vec<u8>,
        msg_type: NetworkMessageType
    ) -> SerializedMessageType {
        let amt_of_chunks = (data.len() + PAYLOAD_DATA_LENGTH - 1) / PAYLOAD_DATA_LENGTH;
        debug_assert!(amt_of_chunks < (u8::MAX as usize), "{}", amt_of_chunks);
        let mut byte_chunks: Vec<Vec<u8>> = Vec::new();
        let mut rng = rand::thread_rng();
        let random_bytes: Vec<u8> = (0..AMT_RANDOM_BYTES).map(|_| rng.gen()).collect(); // First few random bytes (3 bytes in this example)
        for i in 0..amt_of_chunks {
            let mut msg_bytes = Vec::new();
            match msg_type {
                NetworkMessageType::Reliable(seq_num) => {
                    msg_bytes.extend(random_bytes.clone());
                    msg_bytes.push(1); // true
                    msg_bytes.push(seq_num.0.wrapping_add(i as u8));
                    msg_bytes.push(seq_num.0);
                    msg_bytes.push(amt_of_chunks as u8);
                    msg_bytes.push(discriminator_byte);

                    debug_assert!(msg_bytes[RELIABLE_FLAG_BYTE_POS] == 1);
                    debug_assert!(msg_bytes[SEQ_NUM_BYTE_POS] == seq_num.0.wrapping_add(i as u8));
                    debug_assert!(msg_bytes[BASE_CHUNK_SEQ_NUM_BYTE_POS] == seq_num.0);
                    debug_assert!(msg_bytes[AMT_OF_CHUNKS_BYTE_POS] == (amt_of_chunks as u8));
                    debug_assert!(msg_bytes[DISCRIMINANT_BIT_START_POS] == discriminator_byte);
                }
                NetworkMessageType::Unreliable => {
                    panic!("Cannot send chunked message unreliable");
                }
            }
            msg_bytes.extend(
                &data[i * PAYLOAD_DATA_LENGTH..((i + 1) * PAYLOAD_DATA_LENGTH).min(data.len())]
            );
            byte_chunks.push(msg_bytes);
        }
        return SerializedMessageType::from_chunked_msg(byte_chunks);
    }
    pub fn push_non_chunked(bytes: &mut Vec<u8>) {
        bytes.push(0);
        bytes.push(0);
        debug_assert!(bytes[BASE_CHUNK_SEQ_NUM_BYTE_POS] == 0);
        debug_assert!(bytes[AMT_OF_CHUNKS_BYTE_POS] == 0);
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
            NetworkMessageType::Reliable(seq_num) => {
                bytes.push(1); // true
                bytes.push(seq_num.0);
                debug_assert!(bytes[RELIABLE_FLAG_BYTE_POS] == 1);
                debug_assert!(bytes[SEQ_NUM_BYTE_POS] == seq_num.0);
            }
            NetworkMessageType::Unreliable => {
                bytes.push(0);
                bytes.push(0);
                debug_assert!(bytes[RELIABLE_FLAG_BYTE_POS] == 0);
                debug_assert!(bytes[SEQ_NUM_BYTE_POS] == 0);
            }
        }

        match *self {
            Self::ClientSentWorld(ref sim) => {
                if sim.len() > PAYLOAD_DATA_LENGTH {
                    return self.chunk_message(
                        NetworkMessage::ClientSentWorld(Vec::new()).into(),
                        &sim,
                        msg_type
                    );
                } else {
                    Self::push_non_chunked(&mut bytes);
                    bytes.push(NetworkMessage::ClientSentWorld(Vec::new()).into());
                    bytes.extend(sim); // append actual Vec<u8> data
                    return SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                        bytes,
                    });
                }
            }
            Self::ClientSentPlayerInputs(ref inp) => {
                Self::push_non_chunked(&mut bytes);
                bytes.push(NetworkMessage::ClientSentPlayerInputs(Vec::new()).into());
                let packed_inputs = Self::pack_player_inputs(inp);
                bytes.push(packed_inputs);
                SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                    bytes,
                })
            }
            Self::ServerSideAck(ref seq_num) => {
                Self::push_non_chunked(&mut bytes);
                bytes.push(NetworkMessage::ServerSideAck(SeqNum(0)).into());
                bytes.push(seq_num.0);
                SerializedMessageType::from_serialized_msg(SerializedNetworkMessage {
                    bytes,
                })
            }
            Self::ClientSideAck(ref seq_num) => {
                Self::push_non_chunked(&mut bytes);
                bytes.push(NetworkMessage::ClientSideAck(SeqNum(0)).into());
                bytes.push(seq_num.0);
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
            3 => Ok(NetworkMessage::ClientSentPlayerInputs(Vec::new())),

            4 => Ok(NetworkMessage::ServerSideAck(SeqNum(0))),
            5 => Ok(NetworkMessage::ClientSideAck(SeqNum(0))),

            6 => Ok(NetworkMessage::ServerSentPlayerIDs(Vec::new())),
            7 => Ok(NetworkMessage::ServerSentPlayerInputs(Vec::new())),
            8 => Ok(NetworkMessage::ServerSentWorld(Vec::new())),
            9 => Ok(NetworkMessage::ClientConnectToOtherWorld(ServerPlayerID(0))),
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
        const ARRAY_REPEAT_VALUE: Vec<ChunkOfMessage> = Vec::new();
        return ChunkedMessageCollector {
            msgs: [ARRAY_REPEAT_VALUE; (u8::MAX as usize) + 1], // need 256 so that 255 is valid index -> u8:max needs to be a valid index see self.msgs[chunk.seq_num]
        };
    }
    pub fn collect(&mut self, chunk: ChunkOfMessage) {
        self.msgs[chunk.base_seq_num as usize].push(chunk);
    }
    pub fn try_combine(&mut self) -> Option<DeserializedMessage> {
        for msg in &mut self.msgs {
            msg.sort_by_key(|chunk| chunk.seq_num);
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
    pub fn log_simulated_packet_loss(&self, sequence_number: u8) {
        if self.log {
            println!("Simulated packet loss for SeqNum {}", sequence_number);
        }
    }
    pub fn log_received_ack(&self, ack_num: u8) {
        if self.log {
            println!("Received ack from server: {}", ack_num);
        }
    }
    pub fn log_pending_acks(&self, pending: Vec<SeqNum>) {
        if self.log {
            println!("Currently pending acks: {:?}", pending)
        }
    }
    pub fn log_sent_retransmission(&self, seq_num: u8) {
        if self.log {
            println!("Sent retransmission for SeqNum: {}", seq_num);
        }
    }
    pub fn log_sent_packet(&self, seq_num: u8) {
        if self.log {
            println!("Sent packet {}", seq_num);
        }
    }
}
