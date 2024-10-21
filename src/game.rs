use client_conn::ConnectionServer;
use input_buffer::InputBuffer;
use macroquad::prelude::*;
use memory::{ PageAllocator, PAGE_SIZE_BYTES };
use types::{
    Bullet,
    Enemy,
    GameState,
    NetworkedPlayerInput,
    Player,
    PlayerID,
    PlayerInput,
    ServerPlayerID,
    Simulation,
    BULLET_SIZE,
    ENEMY_SIZE,
    MAX_BULLETS,
    MAX_ENEMIES,
    RELOAD_TIME,
};
use crate::types::NetworkMessage;
const PHYSICS_FRAME_TIME: f32 = 1.0 / 60.0;
use ::rand::{ rngs::StdRng, Rng, SeedableRng };
mod types;
mod type_impl;
mod input_buffer;
mod client_conn;
mod memory;
fn simple_hash(frame_number: u32) -> u32 {
    let bytes = frame_number.to_le_bytes();
    let mut hash = 0u32;
    for &byte in &bytes {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(31);
    }

    hash
}

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
            curr_reload_time: 0.0,
        }
    }

    fn update(&mut self, dt: f32) {
        self.position.x += self.movement_input * self.speed * dt;
        self.position.x = self.position.x.clamp(20.0, screen_width() - 20.0);
        self.curr_reload_time += dt;
        if self.shoot_input && self.curr_reload_time > RELOAD_TIME {
            self.curr_reload_time = 0.0;
            if
                let Some(bullet) = self.bullets
                    .iter_mut()
                    .find(|b| (b.position.y <= 0.0 || b.position.y >= screen_height()))
            {
                bullet.position = self.position;
                bullet.velocity = vec2(0.0, -500.0);
            }
        }

        for bullet in &mut self.bullets {
            if bullet.position.y > 0.0 && bullet.position.y < screen_height() {
                bullet.position += bullet.velocity * dt;
            } else {
                bullet.position = vec2(-5.0, -5.0);
            }
        }
    }

    fn draw(&self) {
        draw_rectangle(self.position.x - 20.0, self.position.y - 10.0, 40.0, 20.0, self.color);

        for bullet in &self.bullets {
            draw_circle(bullet.position.x, bullet.position.y, BULLET_SIZE, WHITE);
        }
    }
}
impl Enemy {
    fn new(x: f32, y: f32) -> Self {
        Self {
            position: vec2(x, y),
        }
    }

    fn new_random_at_top(frame: u32) -> Self {
        let seed = simple_hash(frame) as u64;
        let mut rng = StdRng::seed_from_u64(seed);
        Self {
            position: vec2(rng.gen_range(40.0..screen_width() - 40.0), 0.0),
        }
    }

    fn is_active(&self) -> bool {
        self.position.y >= 0.0 && self.position.y < screen_height()
    }

    fn deactivate(&mut self) {
        self.position = vec2(-5.0, -5.0);
    }

    fn update(&mut self, dt: f32) {
        self.position.y += 100.0 * dt;
        if self.position.y >= screen_height() {
            self.deactivate();
        }
    }

    fn draw(&self) {
        if self.is_active() {
            draw_rectangle(
                self.position.x - 20.0,
                self.position.y - 20.0,
                ENEMY_SIZE,
                ENEMY_SIZE,
                RED
            );
        }
    }

    fn update_all(enemies: &mut [Enemy], dt: f32, frame: u32) {
        let mut enemy_cnt = 0;

        for enemy in enemies.iter_mut() {
            if enemy.is_active() {
                enemy_cnt += 1;
                enemy.update(dt);
            }
        }

        // Move active enemies to the front
        enemies.sort_by_key(|enemy| !enemy.is_active());

        if frame % 120 == 0 && enemy_cnt < MAX_ENEMIES {
            enemies[enemy_cnt as usize] = Enemy::new_random_at_top(frame);
        }
    }

    fn check_intersection_bullets(
        enemies: &mut [Enemy],
        bullets: &[Bullet]
    ) -> [bool; MAX_BULLETS] {
        let mut collisions = [false; MAX_BULLETS];
        for enemy in enemies.iter_mut().filter(|e| e.is_active()) {
            for (i, bullet) in bullets.iter().enumerate() {
                if
                    enemy.position.distance(bullet.position) < (BULLET_SIZE + ENEMY_SIZE) / 2.0 &&
                    !collisions[i]
                {
                    enemy.deactivate();
                    collisions[i] = true;
                }
            }
        }

        collisions
    }
    fn draw_all(enemies: &[Enemy]) {
        for enemy in enemies.iter() {
            enemy.is_active();
            {
                enemy.draw();
            }
        }
    }
}

impl Simulation {
    fn new(alloc: &mut PageAllocator) -> Self {
        let player_ptr = alloc
            .alloc_and_write_fixed(&Player::new(100.0, BLUE))
            .expect("Failed to alloc player");
        let player2_ptr = alloc
            .alloc_and_write_fixed(&Player::new(250.0, GREEN))
            .expect("Failed to alloc 2nd player");
        let enemies_arr_ptr = alloc
            .alloc_and_write_fixed(&[Enemy::new(-5.0, -5.0); MAX_ENEMIES as usize])
            .expect("Failed to alloc enemies");
        let frame = alloc.alloc_and_write_fixed(&(0 as u32)).expect("Failed to alloc spawn timer");
        Self {
            player1: player_ptr,
            player2: player2_ptr,
            enemies: enemies_arr_ptr,
            frame: frame,
        }
    }
    fn new_from_serialized(data: Vec<u8>, alloc: &mut PageAllocator) -> Self {
        let sim = Self::new(alloc);
        alloc.set_memory(&data);
        return sim;
    }

    fn update(
        &self,
        dt: f32,
        player_inputs: [Option<Vec<PlayerInput>>; MAX_PLAYER_COUNT as usize],
        alloc: &mut PageAllocator
    ) {
        for (player_id, inputs) in player_inputs.iter().enumerate() {
            if let Some(inputs) = inputs {
                let player_id = PlayerID::from_usize(player_id).unwrap();
                self.handle_player_input(player_id, inputs, alloc);
            }
        }

        let frame = alloc.read_fixed(&self.frame);
        let player1 = alloc.read_fixed(&self.player1);
        let player2 = alloc.read_fixed(&self.player2);

        let enemies = alloc.mut_read_fixed(&self.enemies);
        Enemy::update_all(enemies, dt, frame);
        let player1_bullet_collisions = Enemy::check_intersection_bullets(
            enemies,
            &player1.bullets
        );
        let player2_bullet_collisions = Enemy::check_intersection_bullets(
            enemies,
            &player2.bullets
        );

        let player1 = alloc.mut_read_fixed(&self.player1);
        player1.update(dt);
        for i in 0..player1_bullet_collisions.len() {
            if player1_bullet_collisions[i] {
                player1.bullets[i].position = vec2(-5.0, -5.0);
            }
        }

        let player2 = alloc.mut_read_fixed(&self.player2);
        player2.update(dt);
        for i in 0..player2_bullet_collisions.len() {
            if player2_bullet_collisions[i] {
                player2.bullets[i].position = vec2(-5.0, -5.0);
            }
        }
        let frame = alloc.mut_read_fixed(&self.frame);
        *frame += 1;
    }

    fn draw(&self, local_player_id: PlayerID, other_player_connected: bool, alloc: &PageAllocator) {
        if local_player_id == PlayerID::Player1 {
            alloc.read_fixed(&self.player1).draw();

            if other_player_connected {
                alloc.read_fixed(&self.player2).draw();
            }
        } else {
            alloc.read_fixed(&self.player1).draw();
            alloc.read_fixed(&self.player2).draw();
        }
        let enemies = alloc.read_fixed(&self.enemies);
        Enemy::draw_all(&enemies);
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
pub const MAX_PLAYER_COUNT: u8 = 2;

#[macroquad::main("2 Player Cube Shooter")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut pred_allocator = PageAllocator::new(PAGE_SIZE_BYTES * 5, PAGE_SIZE_BYTES);
    let mut verif_allocator = PageAllocator::new(PAGE_SIZE_BYTES * 5, PAGE_SIZE_BYTES);

    let mut predicted_simulation: Option<Simulation> = None;
    let mut verified_simulation: Option<Simulation> = None;

    let (connection_server, request_sender, server_message_rcv) = ConnectionServer::new()?;
    ConnectionServer::start(connection_server);
    let mut local_player_id = PlayerID::Player1;

    let mut chose_player = false;
    let mut game_state = GameState::ChooseMode;
    let mut other_player_ids: Vec<u8> = Vec::new();
    let mut timer = 0.0;
    let mut input_buffer = InputBuffer::new();
    let mut session_player_count = 1;
    loop {
        clear_background(BLACK);

        match game_state {
            GameState::ChooseMode => {
                draw_text("Choose mode:", 20.0, 40.0, 30.0, WHITE);
                draw_text("Press 'H' to Host", 20.0, 80.0, 20.0, WHITE);
                draw_text("Press 'J' to Join", 20.0, 110.0, 20.0, WHITE);

                if is_key_pressed(KeyCode::H) {
                    verified_simulation = Some(Simulation::new(&mut verif_allocator));
                    predicted_simulation = Some(Simulation::new(&mut pred_allocator));
                    game_state = GameState::Playing;
                } else if is_key_pressed(KeyCode::J) {
                    request_sender.send(
                        types::GameRequestToNetwork::DirectRequest(
                            NetworkMessage::GetServerPlayerIDs
                        )
                    )?;
                    game_state = GameState::WaitingForPlayerList;
                }
            }
            GameState::WaitingForPlayerList => {
                draw_text("Waiting for player list...", 20.0, 40.0, 30.0, WHITE);
                if let Ok(NetworkMessage::ServerSentPlayerIDs(ids)) = server_message_rcv.try_recv() {
                    // println!("received ids {:?}", ids);
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

                for i in 0..9 {
                    if
                        is_key_pressed(keycodes[i as usize]) &&
                        (i as usize) < other_player_ids.len()
                    {
                        let player_to_connect_to: ServerPlayerID = ServerPlayerID(
                            other_player_ids[i as usize]
                        );
                        request_sender.send(
                            types::GameRequestToNetwork::DirectRequest(
                                NetworkMessage::ClientConnectToOtherWorld(player_to_connect_to)
                            )
                        )?;
                        chose_player = true;
                        break;
                    }
                }

                if chose_player {
                    if let Ok(msg) = server_message_rcv.try_recv() {
                        match msg {
                            NetworkMessage::ServerSentPlayerInputs(inputs) => {
                                for input in inputs.buffered_inputs {
                                    let other_player = input.inputs;
                                    println!(
                                        "received inputs while loading |  frame : {:?}",
                                        input.frame
                                    );
                                    input_buffer.insert_other_player_inp(
                                        other_player.clone(),
                                        input.frame
                                    );
                                }
                            }
                            NetworkMessage::ServerSentWorld(data) => {
                                verified_simulation = Some(
                                    Simulation::new_from_serialized(
                                        data.clone(),
                                        &mut verif_allocator
                                    )
                                );
                                predicted_simulation = Some(
                                    Simulation::new_from_serialized(data, &mut pred_allocator)
                                );
                                debug_assert!(
                                    verif_allocator.read_fixed(
                                        &verified_simulation.unwrap().frame
                                    ) ==
                                        pred_allocator.read_fixed(
                                            &predicted_simulation.unwrap().frame
                                        )
                                );
                                debug_assert!(
                                    verif_allocator.read_fixed(
                                        &verified_simulation.unwrap().frame
                                    ) > 0
                                );
                                session_player_count = session_player_count + 1;
                                local_player_id = PlayerID::Player2;
                                game_state = GameState::Playing;
                                input_buffer.update_player_count(
                                    local_player_id,
                                    session_player_count,
                                    verif_allocator.read_fixed(&verified_simulation.unwrap().frame)
                                );
                            }
                            _ =>
                                println!(
                                    "Unexpected message received when waiting for world download"
                                ),
                        }
                    }
                }
            }
            GameState::Playing => {
                if
                    let (Some(ref mut verified_simulation), Some(ref mut predicted_simulation)) = (
                        verified_simulation,
                        predicted_simulation,
                    )
                {
                    let dt = get_frame_time();
                    timer += dt;
                    let mut curr_player = Vec::new();
                    if is_key_down(KeyCode::A) || is_key_down(KeyCode::Left) {
                        curr_player.push(PlayerInput::Left);
                    }
                    if is_key_down(KeyCode::D) || is_key_down(KeyCode::Right) {
                        curr_player.push(PlayerInput::Right);
                    }
                    if is_key_down(KeyCode::W) || is_key_down(KeyCode::Up) {
                        curr_player.push(PlayerInput::Shoot);
                    }
                    if timer >= PHYSICS_FRAME_TIME {
                        timer -= PHYSICS_FRAME_TIME;
                        request_sender.send(
                            types::GameRequestToNetwork::IndirectRequest(
                                types::GameMessage::ClientSentPlayerInputs(
                                    NetworkedPlayerInput::new(curr_player.clone(), if
                                        session_player_count > 1
                                    {
                                        pred_allocator.read_fixed(&predicted_simulation.frame) + 1
                                    } else {
                                        verif_allocator.read_fixed(&verified_simulation.frame)
                                    })
                                )
                            )
                        )?;

                        input_buffer.insert_curr_player_inp(curr_player.clone(), if
                            session_player_count > 1
                        {
                            pred_allocator.read_fixed(&predicted_simulation.frame) + 1
                        } else {
                            verif_allocator.read_fixed(&verified_simulation.frame) + 1
                        });
                        while let Ok(msg) = server_message_rcv.try_recv() {
                            match msg {
                                NetworkMessage::ServerSentPlayerInputs(inputs) => {
                                    for input in inputs.buffered_inputs {
                                        let other_player = input.inputs;
                                        println!("inputs other player for frame {:?}", input.frame);
                                        input_buffer.insert_other_player_inp(
                                            other_player.clone(),
                                            input.frame
                                        );
                                    }
                                }
                                NetworkMessage::ServerRequestHostForWorldData => {
                                    if session_player_count == 1 {
                                        // TODO and player id is not the same as other player
                                        session_player_count += 1;
                                        input_buffer.update_player_count(
                                            local_player_id,
                                            session_player_count,
                                            verif_allocator.read_fixed(&verified_simulation.frame)
                                        ); // start predicting
                                        pred_allocator.set_memory(
                                            &verif_allocator.get_copy_of_state()
                                        );
                                    }
                                    // this also means that we are connecting with someone and its now a mulitplayer lobby
                                    request_sender.send(
                                        types::GameRequestToNetwork::DirectRequest(
                                            NetworkMessage::ClientSentWorld(
                                                verif_allocator.get_copy_of_state()
                                            )
                                        )
                                    )?;

                                    request_sender.send(
                                        types::GameRequestToNetwork::IndirectRequest(
                                            types::GameMessage::ClientSentPlayerInputs(
                                                NetworkedPlayerInput::new(
                                                    curr_player.clone(),
                                                    verif_allocator.read_fixed(
                                                        &verified_simulation.frame
                                                    ) + 1
                                                )
                                            )
                                        )
                                    )?;
                                }
                                _ => {}
                            }
                        }
                        let mut new_verified_state = false;
                        while let Some(verif_frame_input) = input_buffer.pop_next_verified_frame() {
                            if verif_frame_input.inputs[local_player_id as usize].is_none() {
                                request_sender.send(
                                    types::GameRequestToNetwork::IndirectRequest(
                                        types::GameMessage::ClientSentPlayerInputs(
                                            NetworkedPlayerInput::new(
                                                curr_player.clone(),
                                                verif_frame_input.frame
                                            )
                                        )
                                    )
                                )?;
                            }

                            debug_assert!(
                                verif_allocator.read_fixed(&verified_simulation.frame) + 1 ==
                                    verif_frame_input.frame,
                                "verif frame inp {:?}",
                                verif_frame_input
                            );
                            verified_simulation.update(
                                PHYSICS_FRAME_TIME,
                                verif_frame_input.inputs.clone(),
                                &mut verif_allocator
                            );
                            debug_assert!(
                                verif_allocator.read_fixed(&verified_simulation.frame) ==
                                    verif_frame_input.frame
                            );
                            new_verified_state = true;
                        }
                        if new_verified_state && session_player_count > 1 {
                            pred_allocator.set_memory(&verif_allocator.get_copy_of_state());
                        }

                        for (
                            _,
                            pred_frame_input,
                        ) in input_buffer.excluding_iter_after_last_verified() {
                            if
                                pred_allocator.read_fixed(&predicted_simulation.frame) < // by doing this we exclude verified automatically as it would be in the .frame from verified update above
                                pred_frame_input.frame
                                // if we need to play catchup we cannot afford to wait for this players input as we would stay behind
                            {
                                request_sender.send(
                                    types::GameRequestToNetwork::IndirectRequest(
                                        types::GameMessage::ClientSentPlayerInputs(
                                            NetworkedPlayerInput::new(
                                                curr_player.clone(),
                                                pred_frame_input.frame
                                            )
                                        )
                                    )
                                )?;

                                debug_assert!(
                                    pred_allocator.read_fixed(&predicted_simulation.frame) + 1 ==
                                        pred_frame_input.frame,
                                    "curr frame {} vs next frames input {}",
                                    pred_allocator.read_fixed(&predicted_simulation.frame) + 1,
                                    pred_frame_input.frame
                                );
                                predicted_simulation.update(
                                    PHYSICS_FRAME_TIME,
                                    pred_frame_input.inputs.clone(),
                                    &mut pred_allocator
                                );
                                debug_assert!(
                                    pred_allocator.read_fixed(&predicted_simulation.frame) ==
                                        pred_frame_input.frame
                                );
                            }
                        }
                    }

                    if session_player_count > 1 {
                        predicted_simulation.draw(
                            local_player_id,
                            true, // TODO
                            &pred_allocator
                        );
                    } else {
                        verified_simulation.draw(local_player_id, false, &verif_allocator);
                    }

                    draw_text(
                        &format!(
                            "Player is: {:?} | Current verified Frame: {} |  pred frame {} ",
                            local_player_id,
                            verif_allocator.read_fixed(&verified_simulation.frame),
                            pred_allocator.read_fixed(&predicted_simulation.frame)
                        ),
                        25.0,
                        25.0,
                        20.0,
                        WHITE
                    );
                }
            }
        }

        next_frame().await;
    }
}
