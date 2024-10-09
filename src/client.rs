use std::{ io, net::UdpSocket, time::Duration };
use client_conn::ConnectionServer;
use macroquad::prelude::*;
use memory::{ PageAllocator, PAGE_SIZE_BYTES };
use types::{
    Bullet,
    Enemy,
    Player,
    PlayerID,
    PlayerInput,
    ServerPlayerID,
    Simulation,
    SimulationDataMut,
    MAX_BULLETS,
    MAX_ENEMIES,
};
use crate::types::NetworkMessage;
mod types;
mod type_impl;
mod client_conn;
mod memory;
impl Player {
    fn new(x: f32, color: Color) -> Self {
        Self {
            position: vec2(x, screen_height() - 50.0),
            speed: 300.0,
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
        let spawn_timer_ptr = alloc
            .alloc_and_write_fixed(&get_time())
            .expect("Failed to alloc spawn timer");
        Self {
            player1: player_ptr,
            player2: player_ptr,
            enemies: enemies_arr_ptr,
            spawn_timer: spawn_timer_ptr,
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
        let player1 = alloc.mut_read_fixed(&self.player1);
        player1.update(dt);
        let player2 = alloc.mut_read_fixed(&self.player2);
        player2.update(dt);
        let spawn_timer = alloc.mut_read_fixed(&self.spawn_timer);
        if get_time() - *spawn_timer > 1.0 {
            // add enemy

            *spawn_timer = get_time();
        }
    }

    fn draw(&self, alloc: &PageAllocator) {
        alloc.read_fixed(&self.player1).draw();
        alloc.read_fixed(&self.player2).draw();
        for enemy in &alloc.read_fixed(&self.enemies) {
            enemy.draw();
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
        player_to_change.movement_input = 0.0;
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

#[macroquad::main("2 Player Cube Shooter")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut allocator = PageAllocator::new(1024 * 1024 * 1, PAGE_SIZE_BYTES);
    let mut simulation: Simulation;
    let (connection_server, request_sender, mut response_receiver) = ConnectionServer::new()?;
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.spawn(async move {
        connection_server.run().await;
    });
    println!("Do you want to host or join? (h/j)");
    let mut input = String::new();

    loop {
        io::stdin().read_line(&mut input).expect("Failed to read line");
        let choice = input.trim();
        match choice {
            "h" => {
                simulation = Simulation::new(&mut allocator);
                break;
            }
            "j" => {
                request_sender.send(NetworkMessage::GetServerPlayerIDs)?;
                if
                    let Some(NetworkMessage::SendServerPlayerIDs(ids)) =
                        response_receiver.recv().await
                {
                    println!("Received server player ids: {:?}", ids);
                }
                println!("Which player do you want to connect to? Press 0-9 to get the player");
                let other_player_id: ServerPlayerID;
                loop {
                    input.clear(); // Clear previous input
                    io::stdin().read_line(&mut input).expect("Failed to read line");
                    let choice = input.trim();
                    //match choice.parse::<usize>() {
                    //    Ok(index) if index < other_worlds.len() => {
                    //        other_player_id = other_worlds[index];
                    //        break;
                    //    }
                    //    _ => println!("Invalid choice. Please press 0-9 to select a player."),
                    //}
                }
                break;
            }
            _ => {
                println!("Invalid choice, try again!");
                input.clear();
            }
        }
    }
    let mut player1_inputs = Vec::new();
    loop {
        let dt = get_frame_time();

        player1_inputs.clear();
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

        simulation.update(dt, &player1_inputs, &player2_inputs, &mut allocator);

        request_sender.send(NetworkMessage::SendPlayerInputs(player1_inputs.clone()))?;
        clear_background(BLACK);
        simulation.draw(&allocator);
        next_frame().await;
    }
}
