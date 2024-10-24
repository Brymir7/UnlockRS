use std::time::Instant;

use macroquad::{ color::Color, math::Vec2 };
use crate::memory::FixedDataPtr;
pub const MAX_UDP_PAYLOAD_LEN: usize = 508; // https://stackoverflow.com/questions/1098897/what-is-the-largest-safe-udp-packet-size-on-the-internet
pub const MAX_UDP_PAYLOAD_DATA_LENGTH: usize = MAX_UDP_PAYLOAD_LEN - DATA_BIT_START_POS;
pub const MAX_BULLETS: usize = 5;
pub const MAX_ENEMIES: usize = 20;
pub const RELOAD_TIME: f32 = 0.5;
pub const BULLET_SIZE: f32 = 5.0;
pub const ENEMY_SIZE: f32 = 40.0;
pub const AMT_RANDOM_BYTES: usize = 1;
pub const RELIABLE_FLAG_BYTE_POS: usize = AMT_RANDOM_BYTES; // AMT random bytes starts with bit 0 so bit AMT_RANDOM_BYTES - 1 is last bit of it, and AMT_RANDOM_BYTES IS FREE
pub const SEQ_NUM_BYTE_POS: usize = RELIABLE_FLAG_BYTE_POS + 1;

pub const BASE_CHUNK_SEQ_NUM_BYTE_POS: usize = SEQ_NUM_BYTE_POS + 2; // u16
pub const AMT_OF_CHUNKS_BYTE_POS: usize = BASE_CHUNK_SEQ_NUM_BYTE_POS + 2; // u16
pub const DISCRIMINANT_BIT_START_POS: usize = AMT_OF_CHUNKS_BYTE_POS + 2; // u16
pub const DATA_BIT_START_POS: usize = DISCRIMINANT_BIT_START_POS + 1;
pub const PLAYER_MOVE_LEFT_BYTE_POS: usize = 1;
pub const PLAYER_MOVE_RIGHT_BYTE_POS: usize = 2;
pub const PLAYER_SHOOT_BYTE_POS: usize = 3;
pub const VECTOR_LEN_BYTE_POS: usize = DATA_BIT_START_POS;

#[derive(Copy, Clone)]
pub struct Player {
    pub position: Vec2,
    pub speed: f32,
    pub color: Color,
    pub bullets: [Bullet; MAX_BULLETS],
    pub movement_input: f32,
    pub shoot_input: bool,
    pub curr_reload_time: f32,
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
    pub frame: FixedDataPtr<u32>,
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlayerInput {
    Left,
    Right,
    Shoot,
}
#[derive(PartialEq, Copy, Clone, Debug)]
pub enum PlayerID {
    Player1,
    Player2,
}

#[derive(Debug, Copy, Clone)]
pub struct ServerPlayerID(pub u8);

#[derive(Debug, Clone, PartialEq)]
pub struct NetworkedPlayerInput {
    pub inputs: Vec<PlayerInput>,
    pub frame: u32,
}
#[derive(Debug, Clone)]
pub struct BufferedNetworkedPlayerInputs {
    pub buffered_inputs: Vec<NetworkedPlayerInput>,
}

#[repr(u8)]
#[derive(Debug, Clone)]
pub enum NetworkMessage {
    GetServerPlayerIDs = 0,
    GetOwnServerPlayerID = 1,

    ClientSentWorld(Vec<u8>) = 2,
    ClientSentPlayerInputs(BufferedNetworkedPlayerInputs) = 3,

    ServerSideAck(SeqNum) = 4,
    ClientSideAck(SeqNum) = 5,

    ServerSentPlayerIDs(Vec<u8>) = 6,
    ServerSentPlayerInputs(BufferedNetworkedPlayerInputs) = 7,
    ServerSentWorld(Vec<u8>) = 8,

    ClientConnectToOtherWorld(ServerPlayerID) = 9,
    ServerRequestHostForWorldData = 10,
}
pub enum GameMessage {
    ClientSentPlayerInputs(NetworkedPlayerInput),
}
pub enum GameRequestToNetwork {
    DirectRequest(NetworkMessage),
    IndirectRequest(GameMessage),
}
#[derive(Eq, Hash, PartialEq, Debug, Clone, Copy)]
pub struct SeqNum(pub u16);

pub struct SeqNumGenerator {
    pub seq_num: SeqNum,
}
pub enum NetworkMessageType {
    ResendUntilAck(SeqNum),
    SendOnce,
    SendOnceButReceiveAck(SeqNum),
}
#[derive(Debug)]
pub struct DeserializedMessage {
    pub reliable: bool,
    pub seq_num: Option<u16>,
    pub msg: NetworkMessage,
}
#[derive(Debug)]
pub struct ChunkOfMessage {
    pub seq_num: u16,
    pub base_seq_num: u16,
    pub amt_of_chunks: u16,
    pub data_bytes: [u8; MAX_UDP_PAYLOAD_LEN],
}

pub enum DeserializedMessageType {
    NonChunked(DeserializedMessage),
    ChunkOfMessage(ChunkOfMessage),
}

#[derive(Clone, Debug)]
pub struct SerializedNetworkMessage {
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug)]
pub struct ChunkedSerializedNetworkMessage {
    pub bytes: Vec<Vec<u8>>,
}
#[derive(Debug)]
pub enum SerializedMessageType {
    NonChunked(SerializedNetworkMessage),
    Chunked(ChunkedSerializedNetworkMessage),
}
pub struct MsgBuffer(pub [u8; MAX_UDP_PAYLOAD_LEN]);

pub enum GameState {
    ChooseMode,
    WaitingForPlayerList,
    ChoosePlayer,
    Playing,
}

pub struct ChunkedMessageCollector {
    pub msgs: Vec<Vec<ChunkOfMessage>>,
}
#[derive(Debug)]
pub struct MessageHeader {
    pub reliable: bool,
    pub seq_num: Option<SeqNum>,
    pub amt_of_chunks: u16,
    pub base_chunk_seq_num: u16,
    pub is_chunked: bool,
    pub message: NetworkMessage,
}

pub struct PacketParser;

pub struct NetworkLogger {
    pub log: bool,
}

pub enum SendInputsError {
    Disconnected,
    IO(std::io::Error),
}

#[derive(Debug, Clone, Copy)]
pub struct LogConfig {
    pub connection: bool,
    pub world_state: bool,
    pub player_input: bool,
    pub message_handling: bool,
    pub ack: bool,
    pub error: bool,
    pub debug: bool,
}
#[derive(Clone, Copy)]
pub struct Logger {
    pub config: LogConfig,
    pub last_log_time: Option<Instant>,
}
