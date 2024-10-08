use macroquad::{ color::Color, math::Vec2 };

pub const MAX_UDP_PAYLOAD_LEN: usize = 508; // https://stackoverflow.com/questions/1098897/what-is-the-largest-safe-udp-packet-size-on-the-internet
pub struct Player {
    pub position: Vec2,
    pub speed: f32,
    pub color: Color,
    pub bullets: Vec<Bullet>,
    pub movement_input: f32,
    pub shoot_input: bool,
}

pub struct Bullet {
    pub position: Vec2,
    pub velocity: Vec2,
}

pub struct Enemy {
    pub position: Vec2,
}

pub struct Simulation {
    pub player1: Player,
    pub player2: Player,
    pub enemies: Vec<Enemy>,
    pub spawn_timer: f64,
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
pub struct ServerPlayerID(u8);

#[repr(u8)]
#[derive(Debug)]
pub enum NetworkMessage {
    GetServerPlayerIDs = 0,
    GetOwnServerPlayerID = 1,
    GetWorldState = 2,
    SendWorldState(Vec<u8>) = 3,
    GetPlayerInputs = 4,
    SendPlayerInputs(Vec<PlayerInput>) = 5,
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
