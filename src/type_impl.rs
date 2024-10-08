use crate::types::{MsgBuffer, ServerRequest, MAX_UDP_PAYLOAD_LEN};


impl MsgBuffer {
    pub fn default() -> MsgBuffer {
        MsgBuffer([0; MAX_UDP_PAYLOAD_LEN])
    }
    pub fn clear(&mut self) {
        self.0 = [0; MAX_UDP_PAYLOAD_LEN];
    }
}

impl ServerRequest {
    pub fn serialize(&self) -> Vec<u8> {
        let bytes: Vec<u8> = Vec::new();
        match *self {
            Self::SendWorldState(sim) => {

            }
            Self::SendPlayerInputs(inp) => {

            }
            
        }
        return bytes;
    }
}