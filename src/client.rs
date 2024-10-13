use std::{ collections::VecDeque, sync::Arc, time::Duration };

use client_conn::ConnectionServer;
use macroquad::{ input, prelude::*, telemetry::Frame };
use memory::{ PageAllocator, PAGE_SIZE_BYTES };
use types::{
    Bullet,
    Enemy,
    GameState,
    NetworkedPlayerInputs,
    Player,
    PlayerID,
    PlayerInput,
    ServerPlayerID,
    Simulation,
    MAX_BULLETS,
    MAX_ENEMIES,
};
use crate::types::NetworkMessage;
const PHYSICS_FRAME_TIME: f32 = 1.0 / 60.0;
const SENT_PLAYER_STATE_TIME: f32 = 5.0;
mod types;
mod type_impl;
mod client_conn;
mod memory;
impl Player {
    fn new(x: f32, color: Color) -> Self {
        Self {
            position: vec2(x, screen_height() - 50.0),
            speed: 150.0,
            color,
            bullets: [
                Bullet {
                    position: Vec2::new(-5.0, -5.0),
                    velocity: vec2(0.0, 0.0),
                };
                MAX_BULLETS
            ],
            movement_input: 0.0,
            shoot_input: false,
        }
    }

    fn update(&mut self, dt: f32) {
        self.position.x += self.movement_input * self.speed * dt;
        self.position.x = self.position.x.clamp(20.0, screen_width() - 20.0);
        if self.shoot_input {
            // self.bullets.push(Bullet {
            //     position: self.position,
            //     velocity: vec2(0.0, -500.0),
            // });
        }
        for bullet in &mut self.bullets {
            bullet.position += bullet.velocity * dt;
        }
        // self.bullets.retain(|bullet| bullet.position.y > 0.0);
    }

    fn draw(&self) {
        draw_rectangle(self.position.x - 20.0, self.position.y - 10.0, 40.0, 20.0, self.color);

        for bullet in &self.bullets {
            draw_circle(bullet.position.x, bullet.position.y, 5.0, WHITE);
        }
    }
}

impl Enemy {
    fn new(x: f32, y: f32) -> Self {
        Self {
            position: vec2(x, y),
        }
    }
    fn update(&mut self, dt: f32) {
        self.position.y += 100.0 * dt;
    }
    fn draw(&self) {
        draw_rectangle(self.position.x - 20.0, self.position.y - 20.0, 40.0, 40.0, RED);
    }
}

impl Simulation {
    fn new(alloc: &mut PageAllocator) -> Self {
        let player_ptr = alloc
            .alloc_and_write_fixed(&Player::new(100.0, BLUE))
            .expect("Failed to alloc player");
        let enemies_arr_ptr = alloc
            .alloc_and_write_fixed(&[Enemy::new(-5.0, -5.0); MAX_ENEMIES as usize])
            .expect("Failed to alloc enemies");
        let amount_of_enemies = alloc
            .alloc_and_write_fixed(&(0 as u8))
            .expect("Failed to alloc amount of enemies");
        let frame = alloc.alloc_and_write_fixed(&(0 as u32)).expect("Failed to alloc spawn timer");
        Self {
            player1: player_ptr,
            player2: player_ptr,
            amount_of_enemies: amount_of_enemies,
            enemies: enemies_arr_ptr,
            frame: frame,
        }
    }
    fn new_from_serialized(data: Vec<u8>, alloc: &mut PageAllocator) {}

    fn add_player(&self, alloc: &mut PageAllocator) {}

    fn update(
        &self,
        dt: f32,
        player1_inputs: &Vec<PlayerInput>,
        player2_inputs: &Vec<PlayerInput>,
        alloc: &mut PageAllocator
    ) {
        self.handle_player_input(PlayerID::Player1, &player1_inputs, alloc);
        self.handle_player_input(PlayerID::Player2, &player2_inputs, alloc);
        let enemy_amt = alloc.read_fixed(&self.amount_of_enemies);
        let enemies = alloc.mut_read_fixed(&self.enemies);
        for (i, enemy) in enemies.iter_mut().enumerate() {
            if i < (enemy_amt as usize) {
                enemy.update(dt);
            }
        }
        // TODO update enemy_amt
        let player1 = alloc.mut_read_fixed(&self.player1);
        player1.update(dt);
        let player2 = alloc.mut_read_fixed(&self.player2);
        player2.update(dt);
        let frame = alloc.mut_read_fixed(&self.frame);
        if *frame % 60 == 0 {
            // spawn enemy
        }
        *frame += 1;
    }

    fn draw(&self, alloc: &PageAllocator) {
        alloc.read_fixed(&self.player1).draw();
        alloc.read_fixed(&self.player2).draw();
        let enemy_amount = alloc.read_fixed(&self.amount_of_enemies);
        for (i, enemy) in alloc.read_fixed(&self.enemies).iter().enumerate() {
            if i < (enemy_amount as usize) {
                enemy.draw();
            }
        }
    }

    fn handle_player_input(
        &self,
        player: PlayerID,
        inputs: &Vec<PlayerInput>,
        alloc: &mut PageAllocator
    ) {
        let player_to_change: &mut Player;
        match player {
            PlayerID::Player1 => {
                player_to_change = alloc.mut_read_fixed(&self.player1);
            }
            PlayerID::Player2 => {
                player_to_change = alloc.mut_read_fixed(&self.player2);
            }
        }
        player_to_change.shoot_input = false;
        for input in inputs {
            match input {
                PlayerInput::Left => {
                    player_to_change.movement_input = -1.0;
                }
                PlayerInput::Right => {
                    player_to_change.movement_input = 1.0;
                }
                PlayerInput::Shoot => {
                    player_to_change.shoot_input = true;
                }
            }
        }
    }
}
pub const PLAYER_COUNT: u8 = 2;
#[derive(Debug)]
struct PlayerInputs {
    keycode: [Option<Vec<PlayerInput>>; PLAYER_COUNT as usize],
    frame: u32,
}
impl PlayerInputs {
    fn new(curr_player_input: Vec<PlayerInput>, frame: u32) -> Self {
        PlayerInputs {
            keycode: [Some(curr_player_input), None],
            frame,
        }
    }
    fn insert_other_player_input(&mut self, other: Vec<PlayerInput>, player_idx: u8) {
        self.keycode[player_idx as usize] = Some(other);
    }
    fn is_verified(&self) -> bool {
        return self.keycode.iter().all(|key| key.is_some());
    }
}
#[derive(Debug)]
struct InputBuffer {
    inputs: VecDeque<PlayerInputs>,
}

impl InputBuffer {
    fn new() -> Self {
        InputBuffer {
            inputs: VecDeque::new(),
        }
    }

    fn insert_curr_player(&mut self, inp: Vec<PlayerInput>) {
        if self.inputs.is_empty() {
            self.inputs.push_back(PlayerInputs::new(inp, 0));
        } else {
            self.inputs.push_back(PlayerInputs::new(inp, self.inputs.back().unwrap().frame + 1));
        }
    }

    fn insert_networked_player_input(&mut self, inp: Vec<PlayerInput>, frame: u32, player_idx: u8) {
        for input in self.inputs.iter_mut() {
            if input.frame == frame {
                input.insert_other_player_input(inp, player_idx);
                return;
            }
            debug_assert!(
                input.frame <= frame,
                "Inputs are out of order; couldn't find frame despite having searched linearly over all frames until frame"
            );
        }
        panic!("Tried to insert an input for a frame that doesn't exist on this player")
    }

    fn get_first_verified_input(&self) -> Option<&PlayerInputs> {
        self.inputs.iter().find(|input| input.is_verified())
    }
}
#[macroquad::main("2 Player Cube Shooter")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut allocator = PageAllocator::new(PAGE_SIZE_BYTES * 4, PAGE_SIZE_BYTES);
    let mut predicted_simulation: Option<Simulation> = None;
    let mut verified_simulation: Option<Simulation> = None;
    let (connection_server, request_sender, response_receiver) = ConnectionServer::new()?;
    ConnectionServer::start(Arc::clone(&connection_server));

    let mut game_state = GameState::ChooseMode;
    let mut other_player_ids: Vec<u8> = Vec::new();
    let mut timer = 0.0;
    let mut input_buffer = InputBuffer::new();
    loop {
        clear_background(BLACK);

        match game_state {
            GameState::ChooseMode => {
                draw_text("Choose mode:", 20.0, 40.0, 30.0, WHITE);
                draw_text("Press 'H' to Host", 20.0, 80.0, 20.0, WHITE);
                draw_text("Press 'J' to Join", 20.0, 110.0, 20.0, WHITE);

                if is_key_pressed(KeyCode::H) {
                    predicted_simulation = Some(Simulation::new(&mut allocator));
                    game_state = GameState::Playing;
                } else if is_key_pressed(KeyCode::J) {
                    request_sender.send(NetworkMessage::GetServerPlayerIDs)?;
                    game_state = GameState::WaitingForPlayerList;
                }
            }
            GameState::WaitingForPlayerList => {
                draw_text("Waiting for player list...", 20.0, 40.0, 30.0, WHITE);
                if let Ok(NetworkMessage::ServerSentPlayerIDs(ids)) = response_receiver.recv() {
                    println!("received ids {:?}", ids);
                    other_player_ids = ids;
                    game_state = GameState::ChoosePlayer;
                }
            }
            GameState::ChoosePlayer => {
                draw_text("Choose a player to connect to:", 20.0, 40.0, 30.0, WHITE);
                for (i, id) in other_player_ids.iter().enumerate() {
                    draw_text(
                        &format!("Press {} for Player {}", i, id),
                        20.0,
                        80.0 + 30.0 * (i as f32),
                        20.0,
                        WHITE
                    );
                }
                let keycodes = [
                    KeyCode::Key0,
                    KeyCode::Key1,
                    KeyCode::Key2,
                    KeyCode::Key3,
                    KeyCode::Key4,
                    KeyCode::Key5,
                    KeyCode::Key6,
                    KeyCode::Key7,
                    KeyCode::Key8,
                    KeyCode::Key9,
                ];
                let mut chose_player = false;
                for i in 0..9 {
                    if
                        is_key_pressed(keycodes[i as usize]) &&
                        (i as usize) < other_player_ids.len()
                    {
                        let player_to_connect_to: ServerPlayerID = ServerPlayerID(
                            other_player_ids[i as usize]
                        );
                        request_sender.send(
                            NetworkMessage::ClientConnectToOtherWorld(player_to_connect_to)
                        )?;
                        chose_player = true;
                        break;
                    }
                }
                if chose_player {
                    predicted_simulation = Some(Simulation::new(&mut allocator));
                    game_state = GameState::Playing;
                }
            }
            GameState::Playing => {
                if let Some(ref mut predicted_sim) = predicted_simulation {
                    let dt = get_frame_time();
                    timer += dt;
                    let mut player1_inputs = Vec::new();
                    if is_key_down(KeyCode::A) {
                        player1_inputs.push(PlayerInput::Left);
                    }
                    if is_key_down(KeyCode::D) {
                        player1_inputs.push(PlayerInput::Right);
                    }
                    if is_key_pressed(KeyCode::W) {
                        player1_inputs.push(PlayerInput::Shoot);
                    }

                    let mut player2_inputs = Vec::new();
                    if
                        let Ok(NetworkMessage::ServerSentPlayerInputs(inputs)) =
                            response_receiver.recv_timeout(Duration::from_millis(1))
                    {
                        println!("received player inputs");
                        player2_inputs = inputs.inputs;
                        let player_idx = 1;
                        debug_assert!(player_idx < PLAYER_COUNT);
                        input_buffer.insert_networked_player_input(
                            player2_inputs.clone(),
                            inputs.frame,
                            player_idx
                        );
                    }

                    if timer >= PHYSICS_FRAME_TIME {
                        timer -= PHYSICS_FRAME_TIME;
                        input_buffer.insert_curr_player(player1_inputs.clone());
                        predicted_sim.update(dt, &player1_inputs, &player2_inputs, &mut allocator);
                        request_sender.send(
                            NetworkMessage::ClientSentPlayerInputs(
                                NetworkedPlayerInputs::new(
                                    player1_inputs.clone(),
                                    allocator.read_fixed(&predicted_sim.frame)
                                )
                            )
                        )?;
                    }
                    predicted_sim.draw(&allocator);
                }
            }
        }

        next_frame().await;
    }
}
