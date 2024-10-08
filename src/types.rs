use macroquad::{ color::Color, math::Vec2 };
use crate::memory::FixedDataPtr;
pub const MAX_UDP_PAYLOAD_LEN: usize = 508; // https://stackoverflow.com/questions/1098897/what-is-the-largest-safe-udp-packet-size-on-the-internet
pub const MAX_BULLETS: usize = 5;
pub const MAX_ENEMIES: usize = 20;
pub const AMT_RANDOM_BYTES: usize = 3;
pub const RELIABLE_FLAG_BIT_POS: usize = AMT_RANDOM_BYTES ; // AMT random bytes starts with bit 0 so bit AMT_RANDOM_BYTES - 1 is last bit of it, and AMT_RANDOM_BYTES IS FREE
pub const SEQ_NUM_BIT_POS: usize = RELIABLE_FLAG_BIT_POS + 1;
pub const DATA_BIT_START_POS: usize = SEQ_NUM_BIT_POS + 1;
pub const PLAYER_MOVE_LEFT_BIT_POS: usize = 1;
pub const PLAYER_MOVE_RIGHT_BIT_POS: usize = 2;
pub const PLAYER_SHOOT_BIT_POS: usize = 3;

#[derive(Copy, Clone)]
pub struct Player {
    pub position: Vec2,
    pub speed: f32,
    pub color: Color,
    pub bullets: [Bullet; MAX_BULLETS],
    pub movement_input: f32,
    pub shoot_input: bool,
}
#[derive(Copy, Clone)]
pub struct Bullet {
    pub position: Vec2,
    pub velocity: Vec2,
}
#[derive(Copy, Clone)]
pub struct Enemy {
    pub position: Vec2,
}
#[derive(Copy, Clone)]
pub struct Simulation {
    pub player1: FixedDataPtr<Player>,
    pub player2: FixedDataPtr<Player>,
    pub enemies: FixedDataPtr<[Enemy; MAX_ENEMIES]>,
    pub spawn_timer: FixedDataPtr<f64>,
}
pub struct SimulationDataMut<'a> {
    pub player1: &'a mut Player,
    pub player2: &'a mut Player,
    pub enemies: &'a mut [Enemy; MAX_ENEMIES],
    pub spawn_timer: &'a mut f64,
}

pub struct SimulationDataRef<'a> {
    player1: &'a Player,
    player2: &'a Player,
    enemies: &'a [Enemy; MAX_ENEMIES],
    spawn_timer: &'a f64,
}
#[derive(Debug, Clone, Copy)]
pub enum PlayerInput {
    Left,
    Right,
    Shoot,
}
pub enum PlayerID {
    Player1,
    Player2,
}

#[derive(Debug, Copy, Clone)]
pub struct ServerPlayerID(pub u8);

#[repr(u8)]
#[derive(Debug)]
pub enum NetworkMessage {
    GetServerPlayerIDs = 0,
    GetOwnServerPlayerID = 1,
    GetWorldState = 2,
    SendWorldState(Vec<u8>) = 3,
    GetPlayerInputs = 4,
    SendPlayerInputs(Vec<PlayerInput>) = 5,
    ServerSideAck(SeqNum) = 6,
    ClientSideAck(SeqNum) = 7,
    SendServerPlayerIDs(Vec<u8>) = 8,
}
#[derive(Eq, Hash, PartialEq, Debug, Clone, Copy)]
pub struct SeqNum(pub u8);
pub enum NetworkMessageType {
    Reliable(SeqNum),
    Unreliable,
}
#[derive(Debug)]
pub struct ServerSideMessage {
    pub reliable: bool,
    pub seq_num: Option<u8>,
    pub msg: NetworkMessage,
}
#[derive(Clone)]
pub struct SerializedNetworkMessage {
    pub bytes: Vec<u8>,
}
pub struct ChunkedSerializedNetworkMessage {
    pub chunk_num: u16,
    pub bytes: [u8; MAX_UDP_PAYLOAD_LEN],
}
pub struct MsgBuffer(pub [u8; MAX_UDP_PAYLOAD_LEN]);
