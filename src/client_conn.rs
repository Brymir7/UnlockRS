use std::net::UdpSocket;

use crate::types::{ PlayerInput, ServerPlayerID, Simulation };

pub struct ConnectionServer {
    socket: UdpSocket,
    buffer_non_ack_messages: Vec<PlayerInput>,
    other_player_inputs: Vec<PlayerInput>,
}

impl ConnectionServer {
    pub fn new() -> Result<Self, std::io::Error> {
        let socket = UdpSocket::bind("127.0.0.1:0")?;
        let res = socket.connect("127.0.0.1:8080");

        match res {
            Ok(_) => {
                return Ok(ConnectionServer {
                    socket: socket,
                    buffer_non_ack_messages: Vec::new(),
                    other_player_inputs: Vec::new(),
                });
            }
            Err(e) => {
                return Err(e);
            }
        }
    }
    pub fn send_player_world_state(&self, sim: &Simulation) {}
    pub fn get_available_player_worlds(&self) -> Vec<ServerPlayerID> {
        todo!()
    }
    pub fn connect_to_other_world(&self, other_player_id: ServerPlayerID) -> Simulation {
        todo!()
    }
    pub fn update(&mut self, inputs: &Vec<PlayerInput>) {
        self.send_player_inputs(inputs).unwrap();
        self.receive_message().unwrap();
    }
    pub fn send_player_inputs(&self, inputs: &Vec<PlayerInput>) -> Result<(), std::io::Error> {
        //self.socket.send(msg)?;
        Ok(())
    }

    pub fn receive_message(&self) -> Result<Vec<u8>, std::io::Error> {
        // receives messages that have been acknowledged, and also receives other player's inputs
        let mut buf = [0; 1024];
        let amt = self.socket.recv(&mut buf)?;
        Ok(buf[..amt].to_vec())
    }
}
