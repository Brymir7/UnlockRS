use std::{ io, net::UdpSocket };
use client_conn::ConnectionServer;
use macroquad::prelude::*;
use types::{ Bullet, Enemy, Player, PlayerID, PlayerInput, ServerPlayerID, Simulation };
mod types;
mod client_conn;
mod memory;
impl Player {
    fn new(x: f32, color: Color) -> Self {
        Self {
            position: vec2(x, screen_height() - 50.0),
            speed: 300.0,
            color,
            bullets: Vec::new(),
            movement_input: 0.0,
            shoot_input: false,
        }
    }

    fn update(&mut self, dt: f32) {
        self.position.x += self.movement_input * self.speed * dt;

        if self.shoot_input {
            self.bullets.push(Bullet {
                position: self.position,
                velocity: vec2(0.0, -500.0),
            });
        }

        for bullet in &mut self.bullets {
            bullet.position += bullet.velocity * dt;
        }

        self.bullets.retain(|bullet| bullet.position.y > 0.0);
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
    fn new() -> Self {
        Self {
            player1: Player::new(100.0, BLUE),
            player2: Player::new(screen_width() - 100.0, GREEN),
            enemies: Vec::new(),
            spawn_timer: get_time(),
        }
    }

    fn update(
        &mut self,
        dt: f32,
        player1_inputs: Vec<PlayerInput>,
        player2_inputs: Vec<PlayerInput>
    ) {
        self.handle_player_input(PlayerID::Player1, &player1_inputs);
        self.handle_player_input(PlayerID::Player2, &player2_inputs);

        self.player1.update(dt);
        self.player2.update(dt);

        if get_time() - self.spawn_timer > 1.0 {
            self.enemies.push(Enemy::new(rand::gen_range(20.0, screen_width() - 20.0), -40.0));
            self.spawn_timer = get_time();
        }

        for enemy in &mut self.enemies {
            enemy.update(dt);
        }

        for player in [&mut self.player1, &mut self.player2].iter_mut() {
            player.bullets.retain(|bullet| {
                let mut hit = false;
                self.enemies.retain(|enemy| {
                    if bullet.position.distance(enemy.position) < 20.0 {
                        hit = true;
                        false
                    } else {
                        true
                    }
                });
                !hit
            });
        }

        if self.enemies.iter().any(|e| e.position.y > screen_height()) {
            draw_text(
                "Game Over!",
                screen_width() / 2.0 - 100.0,
                screen_height() / 2.0,
                50.0,
                WHITE
            );
        }
    }

    fn draw(&self) {
        self.player1.draw();
        self.player2.draw();
        for enemy in &self.enemies {
            enemy.draw();
        }
    }

    fn handle_player_input(&mut self, player: PlayerID, inputs: &Vec<PlayerInput>) {
        let player_to_change: &mut Player;
        match player {
            PlayerID::Player1 => {
                player_to_change = &mut self.player1;
            }
            PlayerID::Player2 => {
                player_to_change = &mut self.player2;
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
async fn main() {
    let mut simulation: Simulation;
    let connection = ConnectionServer::new().expect("Failed to connect to server, try again!");
    println!("Do you want to host or join? (h/j)");
    let mut input = String::new();
    loop {
        io::stdin().read_line(&mut input).expect("Failed to read line");
        let choice = input.trim();
        match choice {
            "h" => {
                simulation = Simulation::new();
                break;
            }
            "j" => {
                let other_worlds = connection.get_available_player_worlds();
                println!("{:?}", other_worlds);
                println!("Which player do you want to connect to? Press 0-9 to get the player");
                let other_player_id: ServerPlayerID;

                loop {
                    input.clear(); // Clear previous input
                    io::stdin().read_line(&mut input).expect("Failed to read line");
                    let choice = input.trim();
                    match choice.parse::<usize>() {
                        Ok(index) if index < other_worlds.len() => {
                            other_player_id = other_worlds[index];
                            break;
                        }
                        _ => println!("Invalid choice. Please press 0-9 to select a player."),
                    }
                }
                simulation = connection.connect_to_other_world(other_player_id);
                break;
            }
            _ => {
                println!("Invalid choice, try again!");
                input.clear();
            }
        }
    }

    loop {
        let dt = get_frame_time();
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
        simulation.update(dt, player1_inputs, player2_inputs);

        clear_background(BLACK);
        simulation.draw();

        next_frame().await;
    }
}
