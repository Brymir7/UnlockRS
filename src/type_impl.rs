use crate::types::{
    MsgBuffer,
    NetworkMessage,
    NetworkMessageType,
    Player,
    PlayerInput,
    SeqNum,
    SerializedNetworkMessage,
    ServerSideMessage,
    AMT_RANDOM_BYTES,
    DATA_BIT_START_POS,
    DISCRIMINANT_BIT_START_POS,
    MAX_UDP_PAYLOAD_LEN,
    PLAYER_MOVE_LEFT_BYTE_POS,
    PLAYER_MOVE_RIGHT_BYTE_POS,
    PLAYER_SHOOT_BYTE_POS,
    RELIABLE_FLAG_BYTE_POS,
    SEQ_NUM_BYTE_POS,
    VECTOR_LEN_BYTE_POS,
};

impl MsgBuffer {
    pub fn default() -> MsgBuffer {
        MsgBuffer([0; MAX_UDP_PAYLOAD_LEN])
    }
    pub fn clear(&mut self) {
        self.0 = [0; MAX_UDP_PAYLOAD_LEN];
    }
    pub fn parse_on_server(&self) -> Result<ServerSideMessage, &'static str> {
        let bytes = &self.0;
        if bytes.is_empty() {
            return Err("Empty buffer");
        }
        println!("{:?}", bytes[..6].to_vec());
        let reliable = bytes[RELIABLE_FLAG_BYTE_POS] > 0;
        let seq_num = if reliable { Some(SeqNum(bytes[SEQ_NUM_BYTE_POS])) } else { None };
        let discriminator = bytes[DISCRIMINANT_BIT_START_POS];
        let message = NetworkMessage::try_from(discriminator)?;
        let data = bytes[DATA_BIT_START_POS..].to_vec();
        let parsed_message = match message {
            NetworkMessage::SendWorldState(_) => { NetworkMessage::SendWorldState(data) }
            NetworkMessage::SendPlayerInputs(_) => {
                let player_inputs = Self::parse_player_inputs(&data);
                NetworkMessage::SendPlayerInputs(player_inputs)
            }
            _ => message,
        };

        let server_side_message = if reliable {
            ServerSideMessage::from_reliable_msg(
                parsed_message,
                seq_num.map(|s| s.0)
            )
        } else {
            ServerSideMessage::from_unreliable_msg(parsed_message)
        };
        Ok(server_side_message)
    }
    pub fn parse_on_client(&self) -> Result<NetworkMessage, &'static str> {
        let bytes = &self.0;

        if bytes.is_empty() {
            return Err("Empty buffer");
        }
        let data = bytes[DATA_BIT_START_POS..].to_vec();
        let discriminator = bytes[DISCRIMINANT_BIT_START_POS];
        let request = NetworkMessage::try_from(discriminator)?;
        match request {
            NetworkMessage::SendWorldState(_) => {
                let data = data; // Extract remaining bytes as Vec<u8>
                Ok(NetworkMessage::SendWorldState(data))
            }
            NetworkMessage::SendPlayerInputs(_) => {
                let player_inputs = Self::parse_player_inputs(&data);
                Ok(NetworkMessage::SendPlayerInputs(player_inputs))
            }
            NetworkMessage::SendServerPlayerIDs(_) => {
                let len = bytes[VECTOR_LEN_BYTE_POS];
                let ids: Vec<u8> =
                    bytes[VECTOR_LEN_BYTE_POS + 1..VECTOR_LEN_BYTE_POS + 1 + (len as usize)].into();
                Ok(NetworkMessage::SendServerPlayerIDs(ids))
            }
            NetworkMessage::ServerSideAck(_) => {
                let seq_num = data[0];
                Ok(NetworkMessage::ServerSideAck(SeqNum(seq_num)))
            }
            _ => Ok(request),
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
}
impl ServerSideMessage {
    fn from_reliable_msg(msg: NetworkMessage, seq_num: Option<u8>) -> Self {
        ServerSideMessage {
            reliable: true,
            seq_num,
            msg: msg,
        }
    }
    fn from_unreliable_msg(msg: NetworkMessage) -> Self {
        ServerSideMessage {
            reliable: false,
            seq_num: None,
            msg: msg,
        }
    }
}
use rand::{ Rng };
impl NetworkMessage {
    pub fn serialize(&self, msg_type: NetworkMessageType) -> SerializedNetworkMessage {
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
            Self::SendWorldState(ref sim) => {
                bytes.push(NetworkMessage::SendWorldState(Vec::new()).into());
                bytes.extend(sim); // append actual Vec<u8> data
            }
            Self::SendPlayerInputs(ref inp) => {
                bytes.push(NetworkMessage::SendPlayerInputs(Vec::new()).into());
                let packed_inputs = Self::pack_player_inputs(inp);
                bytes.push(packed_inputs);
            }
            Self::ServerSideAck(ref seq_num) => {
                bytes.push(NetworkMessage::ServerSideAck(SeqNum(0)).into());
                bytes.push(seq_num.0); // if server sends this, we cannot use the same ACK sequence number that we use for sending from client, because the ACK can also fail
            }
            Self::ClientSideAck(ref seq_num) => {
                bytes.push(NetworkMessage::ClientSideAck(SeqNum(0)).into());
                bytes.push(seq_num.0);
            }
            Self::SendServerPlayerIDs(ref ids) => {
                bytes.push(NetworkMessage::SendServerPlayerIDs(Vec::new()).into());
                debug_assert!(ids.len() <= (u8::MAX as usize));
                bytes.push(ids.len() as u8);
                bytes.extend(ids);
                debug_assert!(bytes[VECTOR_LEN_BYTE_POS] == (ids.len() as u8));
            }
            _ => {
                bytes.push(self.into());
            }
        }

        SerializedNetworkMessage {
            bytes: bytes,
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
            NetworkMessage::GetWorldState => 2,
            NetworkMessage::SendWorldState(_) => 3,
            NetworkMessage::GetPlayerInputs => 4,
            NetworkMessage::SendPlayerInputs(_) => 5,
            NetworkMessage::ServerSideAck(_) => 6,
            NetworkMessage::ClientSideAck(_) => 7,
            NetworkMessage::SendServerPlayerIDs(_) => 8,
            NetworkMessage::ConnectToOtherWorld(_, _) => 9,
        }
    }
}
impl From<&NetworkMessage> for u8 {
    fn from(request: &NetworkMessage) -> u8 {
        match request {
            NetworkMessage::GetServerPlayerIDs => 0,
            NetworkMessage::GetOwnServerPlayerID => 1,
            NetworkMessage::GetWorldState => 2,
            NetworkMessage::SendWorldState(_) => 3,
            NetworkMessage::GetPlayerInputs => 4,
            NetworkMessage::SendPlayerInputs(_) => 5,
            NetworkMessage::ServerSideAck(_) => 6,
            NetworkMessage::ClientSideAck(_) => 7,
            NetworkMessage::SendServerPlayerIDs(_) => 8,
            NetworkMessage::ConnectToOtherWorld(_, _) => 9,
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
            2 => Ok(NetworkMessage::GetWorldState),
            3 => Ok(NetworkMessage::SendWorldState(Vec::new())), // Provide default or placeholder
            4 => Ok(NetworkMessage::GetPlayerInputs),
            5 => Ok(NetworkMessage::SendPlayerInputs(Vec::new())), // Provide default or placeholder
            6 => Ok(NetworkMessage::ServerSideAck(SeqNum(0))),
            7 => Ok(NetworkMessage::ClientSideAck(SeqNum(0))),
            8 => Ok(NetworkMessage::SendServerPlayerIDs(Vec::new())),
            _ => Err("Invalid network msg u8 type"),
        }
    }
}
