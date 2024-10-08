use macroquad::{ color::Color, math::Vec2 };
use crate::memory::FixedDataPtr;
pub const MAX_UDP_PAYLOAD_LEN: usize = 508; // https://stackoverflow.com/questions/1098897/what-is-the-largest-safe-udp-packet-size-on-the-internet
pub const MAX_BULLETS: usize = 5;
pub const MAX_ENEMIES: usize = 20;
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
