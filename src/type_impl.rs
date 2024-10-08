use crate::types::{
    MsgBuffer,
    Player,
    PlayerInput,
    SerializedNetworkMessage,
    NetworkMessage,
    MAX_UDP_PAYLOAD_LEN,
};

impl MsgBuffer {
    pub fn default() -> MsgBuffer {
        MsgBuffer([0; MAX_UDP_PAYLOAD_LEN])
    }
    pub fn clear(&mut self) {
        self.0 = [0; MAX_UDP_PAYLOAD_LEN];
    }
    pub fn parse_message(&self) -> Result<NetworkMessage, &'static str> {
        let bytes = &self.0;

        if bytes.is_empty() {
            return Err("Empty buffer");
        }
        let discriminator = bytes[0];
        let request = NetworkMessage::try_from(discriminator)?;
        match request {
            NetworkMessage::SendWorldState(_) => {
                let data = bytes[1..].to_vec(); // Extract remaining bytes as Vec<u8>
                Ok(NetworkMessage::SendWorldState(data))
            }
            NetworkMessage::SendPlayerInputs(_) => {
                let player_inputs = Self::parse_player_inputs(&bytes[1..]);
                Ok(NetworkMessage::SendPlayerInputs(player_inputs))
            }
            _ => Ok(request), // For variants without data
        }
    }
    fn parse_player_inputs(bytes: &[u8]) -> Vec<PlayerInput> {
        let mut res = Vec::new();
        let byte = bytes[0];
        let player_moves_left = (byte >> 1) & 1;
        let player_moves_right: u8 = (byte >> 2) & 1;
        let player_shoots: u8 = (byte >> 3) & 1;
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
}
use rand::Rng;
impl NetworkMessage {
    pub fn serialize(&self) -> SerializedNetworkMessage {
        let mut rng = rand::thread_rng();
        let mut bytes: Vec<u8> = (0..3).map(|_| rng.gen()).collect(); // First few random bytes (3 bytes in this example)

        match *self {
            Self::SendWorldState(ref sim) => {
                bytes.push(NetworkMessage::SendWorldState as u8); // enum bit
                bytes.extend(sim); // append actual Vec<u8> data
            }
            Self::SendPlayerInputs(ref inp) => {
                bytes.push(NetworkMessage::SendPlayerInputs as u8); // enum bit
                let packed_inputs = Self::pack_player_inputs(inp);
                bytes.push(packed_inputs); // append packed inputs
            }
            _ => {
                bytes.push(u8::from(self));
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
                    res = res + (1 << 1);
                }
                PlayerInput::Right => {
                    res = res + (1 << 2);
                }
                PlayerInput::Shoot => {
                    res = res + (1 << 3);
                }
            }
        }
        return res;
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
            _ => Err("Unknown value for NetworkMessage"),
        }
    }
}
